"""Isolated model of publication-scoped extraction-fact caching.

This deliberately does not invoke or modify the production indexer. Facts are
tiny declaration records; the cache mechanics and invalidation boundaries are
the subject of this probe.
"""
from __future__ import annotations

from collections import OrderedDict
from dataclasses import dataclass
import hashlib
import json
import re
import unittest


PARSER_VERSION = "toy-decl-parser-v1"


class Cancelled(Exception):
    pass


@dataclass(frozen=True)
class Key:
    digest: str
    language: str
    parser: str
    config: str


class FactsCache:
    def __init__(self, max_entries: int = 8, max_bytes: int = 4096):
        self.max_entries = max_entries
        self.max_bytes = max_bytes
        self.bytes = 0
        self.entries: OrderedDict[Key, tuple[dict, int]] = OrderedDict()
        self.extract_calls = 0

    @staticmethod
    def key(source: bytes, language: str, parser: str, config: dict) -> Key:
        config_bytes = json.dumps(config, sort_keys=True, separators=(",", ":")).encode()
        return Key(hashlib.sha256(source).hexdigest(), language, parser,
                   hashlib.sha256(config_bytes).hexdigest())

    def get_or_extract(self, source: bytes, language: str, parser: str,
                       config: dict, extract, cancelled=lambda: False) -> dict:
        key = self.key(source, language, parser, config)
        if key in self.entries:
            value, size = self.entries.pop(key)
            self.entries[key] = (value, size)
            return value
        if cancelled():
            raise Cancelled()
        self.extract_calls += 1
        # Extract into a local value: errors/cancellation cannot publish a partial entry.
        value = extract(source, language, config, cancelled)
        if cancelled():
            raise Cancelled()
        size = len(json.dumps(value, sort_keys=True, separators=(",", ":")).encode())
        if size <= self.max_bytes:
            while self.entries and (len(self.entries) >= self.max_entries or
                                    self.bytes + size > self.max_bytes):
                _, (_, old_size) = self.entries.popitem(last=False)
                self.bytes -= old_size
            if self.bytes + size <= self.max_bytes:
                self.entries[key] = (value, size)
                self.bytes += size
        return value


def toy_extract(source: bytes, language: str, config: dict, cancelled) -> dict:
    """Tiny deterministic stand-in; path-specific IDs are materialized separately."""
    if language not in {"typescript", "javascript"}:
        raise ValueError("unsupported language")
    text = source.decode("utf-8")
    if "/*FAIL*/" in text:
        raise ValueError("synthetic parser error")
    facts = []
    for i, match in enumerate(re.finditer(r"\b(?:class|function)\s+(\w+)", text)):
        if cancelled():
            raise Cancelled()
        facts.append({"kind": "declaration", "name": match.group(1), "ordinal": i})
    # Config affects facts to make configuration invalidation observable.
    if config.get("includeFunctions") is False:
        facts = [fact for fact in facts if "function" not in text]
    return {"facts": facts}


def materialize(facts: dict, path: str) -> list[dict]:
    """Production extraction embeds path in IDs/qualified names (decode.rs)."""
    return [{**fact, "id": f"{path}::{fact['kind']}::{fact['name']}",
             "qualifiedName": f"{path}::{fact['name']}", "filePath": path}
            for fact in facts["facts"]]


class CacheProbe(unittest.TestCase):
    def setUp(self):
        self.cache = FactsCache(max_entries=2, max_bytes=180)
        self.source = b"export class Alpha {}"
        self.config = {"includeFunctions": True}

    def lookup(self, source=None, lang="typescript", parser=PARSER_VERSION, config=None,
               extract=toy_extract, cancelled=lambda: False):
        return self.cache.get_or_extract(source or self.source, lang, parser,
                                         self.config if config is None else config,
                                         extract, cancelled)

    def test_reuses_facts_but_rebinds_path_dependent_ids(self):
        facts_a = self.lookup()
        facts_b = self.lookup()
        self.assertIs(facts_a, facts_b)
        self.assertEqual(self.cache.extract_calls, 1)
        self.assertNotEqual(materialize(facts_a, "src/a.ts")[0]["id"],
                            materialize(facts_b, "src/renamed.ts")[0]["id"])

    def test_content_language_parser_and_config_are_key_dimensions(self):
        self.lookup()
        self.lookup(lang="javascript")
        self.lookup(parser="toy-decl-parser-v2")
        self.lookup(config={"includeFunctions": False})
        self.lookup(source=b"export class Beta {}")
        self.assertEqual(self.cache.extract_calls, 5)

    def test_cancelled_and_failed_extractions_do_not_publish(self):
        calls = 0
        is_cancelled = False
        def cancel_during(source, language, config, cancelled):
            nonlocal calls
            nonlocal is_cancelled
            calls += 1
            is_cancelled = True
            return {"facts": [{"name": "partial"}]}
        with self.assertRaises(Cancelled):
            self.lookup(extract=cancel_during, cancelled=lambda: is_cancelled)
        self.assertEqual(len(self.cache.entries), 0)
        with self.assertRaises(ValueError):
            self.lookup(source=b"/*FAIL*/", extract=toy_extract)
        self.assertEqual(len(self.cache.entries), 0)
        self.assertEqual(calls, 1)

    def test_entry_count_and_byte_budget_evict_oldest(self):
        self.lookup(source=b"class One {}")
        self.lookup(source=b"class Two {}")
        self.lookup(source=b"class Three {}")
        self.assertLessEqual(len(self.cache.entries), 2)
        self.assertLessEqual(self.cache.bytes, self.cache.max_bytes)
        before = self.cache.extract_calls
        self.lookup(source=b"class One {}")
        self.assertEqual(self.cache.extract_calls, before + 1)

    def test_oversize_fact_is_returned_but_not_cached(self):
        cache = FactsCache(max_entries=2, max_bytes=1)
        extractor_calls = 0
        def extract(*args):
            nonlocal extractor_calls
            extractor_calls += 1
            return {"facts": ["larger than one byte"]}
        for _ in range(2):
            self.assertTrue(cache.get_or_extract(b"x", "ts", "v1", {}, extract)["facts"])
        self.assertEqual(extractor_calls, 2)
        self.assertEqual(len(cache.entries), 0)


if __name__ == "__main__":
    unittest.main(verbosity=2)
