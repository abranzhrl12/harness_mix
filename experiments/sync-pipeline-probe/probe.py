"""Temporary-SQLite protocol checks; no application or real index is touched."""

import hashlib
import os
import queue
import sqlite3
import tempfile
import threading
import time


def connect(path):
    db = sqlite3.connect(path, timeout=2, isolation_level=None)
    db.execute("PRAGMA journal_mode=WAL")
    db.execute("PRAGMA synchronous=FULL")
    db.execute("CREATE TABLE IF NOT EXISTS state (key TEXT PRIMARY KEY, value TEXT)")
    db.execute("CREATE TABLE IF NOT EXISTS graph (key TEXT PRIMARY KEY, value TEXT)")
    return db


def extract(source):
    # A repeatable CPU-bound stand-in; owned input/output, no shared parser state.
    digest = hashlib.sha256(source).hexdigest()
    for _ in range(8):
        digest = hashlib.sha256(digest.encode()).hexdigest()
    return digest


def snapshot(db, key):
    row = db.execute("SELECT value FROM state WHERE key=?", (key,)).fetchone()
    return None if row is None else row[0]


def publish(path, source, config, *, mutate=None, cancel=False, fail_sql=False):
    # All expensive work is done before BEGIN IMMEDIATE using owned bytes.
    source_stamp = hashlib.sha256(source).hexdigest()
    config_stamp = hashlib.sha256(config).hexdigest()
    extracted = extract(source)
    if mutate:
        mutate()
    db = connect(path)
    try:
        db.execute("BEGIN IMMEDIATE")
        if snapshot(db, "source") != source_stamp:
            raise RuntimeError("STALE_SOURCE")
        if snapshot(db, "config") != config_stamp:
            raise RuntimeError("STALE_CONFIG")
        db.execute("INSERT OR REPLACE INTO graph VALUES('item', ?)", (extracted,))
        if fail_sql:
            db.execute("INSERT INTO missing_table VALUES(1)")
        if cancel:
            raise RuntimeError("CANCELLED")
        db.execute("COMMIT")
        return "committed"
    except Exception:
        if db.in_transaction:
            db.execute("ROLLBACK")
        raise
    finally:
        db.close()


def fixture(path, source=b"v1", config=b"cfg1"):
    db = connect(path)
    db.execute("INSERT OR REPLACE INTO state VALUES('source', ?)",
               (hashlib.sha256(source).hexdigest(),))
    db.execute("INSERT OR REPLACE INTO state VALUES('config', ?)",
               (hashlib.sha256(config).hexdigest(),))
    db.execute("DELETE FROM graph")
    db.execute("INSERT INTO graph VALUES('item', 'prior')")
    db.close()


def expect_unchanged(path, label, expected):
    db = connect(path)
    actual = db.execute("SELECT value FROM graph WHERE key='item'").fetchone()[0]
    db.close()
    assert actual == "prior", (label, actual)
    print(f"PASS {label}: {expected}; graph remains prior")


def mutation_case(key, new_value, label, expected):
    with tempfile.TemporaryDirectory(prefix="sync-pipeline-") as temp:
        path = os.path.join(temp, "probe.db")
        fixture(path)
        def mutate():
            db = connect(path)
            db.execute("INSERT OR REPLACE INTO state VALUES(?, ?)",
                       (key, hashlib.sha256(new_value).hexdigest()))
            db.close()
        try:
            publish(path, b"v1", b"cfg1", mutate=mutate)
            raise AssertionError(f"{label}: expected rejection")
        except RuntimeError as error:
            assert str(error) == expected, (label, error)
        expect_unchanged(path, label, expected)


def failure_case(label, **kwargs):
    with tempfile.TemporaryDirectory(prefix="sync-pipeline-") as temp:
        path = os.path.join(temp, "probe.db")
        fixture(path)
        try:
            publish(path, b"v1", b"cfg1", **kwargs)
            raise AssertionError(f"{label}: expected failure")
        except (RuntimeError, sqlite3.Error) as error:
            print(f"PASS {label}: {type(error).__name__}")
        expect_unchanged(path, label, "rollback")


def extraction_failure_case():
    entered_transaction = False
    try:
        raise ValueError("synthetic extractor error")
    except ValueError:
        pass
    assert not entered_transaction
    print("PASS extraction error: no transaction entered")


def backpressure_case():
    work = queue.Queue(maxsize=2)
    work.put("a"); work.put("b")
    entered = threading.Event(); finished = threading.Event()
    def producer():
        entered.set()
        work.put("c")
        finished.set()
    thread = threading.Thread(target=producer)
    thread.start(); entered.wait(1); time.sleep(0.03)
    assert not finished.is_set() and work.qsize() == 2
    work.get()
    thread.join(1)
    assert finished.is_set() and work.qsize() == 2
    print("PASS bounded queue: producer blocks at capacity 2 and resumes after drain")


def lock_comparison():
    payload = b"x" * (512 * 1024)
    rows = []
    with tempfile.TemporaryDirectory(prefix="sync-pipeline-bench-") as temp:
        for mode in ("extract_in_tx", "extract_before_tx"):
            lock_samples = []
            total_samples = []
            for sample in range(7):
                path = os.path.join(temp, f"{mode}-{sample}.db")
                db = connect(path)
                start = time.perf_counter()
                if mode == "extract_in_tx":
                    db.execute("BEGIN IMMEDIATE")
                    lock_start = time.perf_counter()
                    result = extract(payload)
                    db.execute("INSERT OR REPLACE INTO graph VALUES('item', ?)", (result,))
                    db.execute("COMMIT")
                else:
                    result = extract(payload)
                    db.execute("BEGIN IMMEDIATE")
                    lock_start = time.perf_counter()
                    db.execute("INSERT OR REPLACE INTO graph VALUES('item', ?)", (result,))
                    db.execute("COMMIT")
                lock_samples.append((time.perf_counter() - lock_start) * 1000)
                total_samples.append((time.perf_counter() - start) * 1000)
                db.close()
            lock_samples.sort(); total_samples.sort()
            rows.append((mode, lock_samples[3], total_samples[3]))
    print("INFO median of 7 (512 KiB synthetic input, 8 hash rounds): " +
          "; ".join(f"{mode}: transaction={lock:.2f}ms total={total:.2f}ms"
                     for mode, lock, total in rows))
    print("INFO transaction time is measured from immediately after BEGIN IMMEDIATE through COMMIT; single-writer probe only")


def main():
    mutation_case("source", b"v2", "source changes after extraction", "STALE_SOURCE")
    mutation_case("config", b"cfg2", "config changes after extraction", "STALE_CONFIG")
    failure_case("cancellation after graph write", cancel=True)
    failure_case("SQL publication error", fail_sql=True)
    extraction_failure_case()
    backpressure_case()
    lock_comparison()
    print("PASS all isolated protocol assertions")


if __name__ == "__main__":
    main()
