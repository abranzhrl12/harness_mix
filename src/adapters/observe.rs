//! A bounded view of retained job output; no arbitrary log paths or commands.
use super::{TaskManager, persistence_error};
use harness_domain::{Failure, assistance::ObserveArgs};
use regex::{Regex, RegexBuilder};
use serde_json::{Value, json};
use std::collections::BTreeSet;

struct Line {
    stream: &'static str,
    text: String,
    cursor: u64,
    partial: bool,
}

fn settings(args: &ObserveArgs) -> Result<Option<Regex>, Failure> {
    if args.task_id.is_empty()
        || args.task_id.len() > 128
        || !(1..=200).contains(&args.lines.unwrap_or(20))
        || args.before.unwrap_or(2) > 20
        || args.after.unwrap_or(2) > 20
        || !(256..=16000).contains(&args.max_chars.unwrap_or(6000))
        || !matches!(
            args.stream.as_deref().unwrap_or("both"),
            "both" | "stdout" | "stderr"
        )
    {
        return Err((
            "ARGUMENT",
            "Invalid taskId, lines, context, stream or maxChars",
        ));
    }
    match &args.filter {
        None => Ok(None),
        Some(pattern) if pattern.len() > 1024 => Err(("ARGUMENT", "filter exceeds 1024 bytes")),
        Some(pattern) => RegexBuilder::new(pattern)
            .size_limit(64 * 1024)
            .dfa_size_limit(1024 * 1024)
            .build()
            .map(Some)
            .map_err(|_| ("ARGUMENT", "Invalid or oversized regex filter")),
    }
}

fn assemble(events: &[Value], stream: &'static str) -> Vec<Line> {
    let mut result = Vec::new();
    let mut pending = String::new();
    let mut last = 0;
    for event in events {
        if event["kind"] == "state" {
            if !pending.is_empty() {
                result.push(Line {
                    stream,
                    text: std::mem::take(&mut pending),
                    cursor: last,
                    partial: true,
                });
            }
            continue;
        }
        if event["kind"] != "output" || event["data"]["stream"] != stream {
            continue;
        }
        last = event["cursor"].as_u64().unwrap_or(0);
        let Some(text) = event["data"]["text"].as_str() else {
            continue;
        };
        for piece in text.split_inclusive('\n') {
            pending.push_str(piece);
            if piece.ends_with('\n') {
                result.push(Line {
                    stream,
                    text: std::mem::take(&mut pending),
                    cursor: last,
                    partial: false,
                });
            }
        }
    }
    if !pending.is_empty() {
        result.push(Line {
            stream,
            text: pending,
            cursor: last,
            partial: true,
        });
    }
    result
}

fn select(events: &[Value], args: &ObserveArgs, regex: Option<&Regex>) -> Value {
    let mut records = Vec::new();
    for stream in ["stdout", "stderr"] {
        if args.stream.as_deref().unwrap_or("both") != "both"
            && args.stream.as_deref() != Some(stream)
        {
            continue;
        }
        records.extend(assemble(events, stream));
    }
    // Reassemble before applying the cursor: a matching line can span two polls.
    records.retain(|line| line.cursor > args.cursor.unwrap_or(0));
    // Context follows each stream, so stderr stack frames are not displaced by stdout.
    let mut chosen = Vec::<(usize, bool)>::new();
    let mut matches: Vec<_> = records
        .iter()
        .enumerate()
        .filter(|(_, line)| regex.is_none_or(|r| r.is_match(&line.text)))
        .map(|(i, _)| i)
        .collect();
    matches.sort_by_key(|i| (records[*i].cursor, *i));
    let count = matches.len();
    for &i in matches.iter().rev().take(args.lines.unwrap_or(20)) {
        for j in i.saturating_sub(args.before.unwrap_or(2))
            ..=(i + args.after.unwrap_or(2)).min(records.len().saturating_sub(1))
        {
            if records[j].stream == records[i].stream {
                chosen.push((j, j == i));
            }
        }
    }
    let matched: BTreeSet<_> = matches.iter().copied().collect();
    let mut indices: Vec<_> = chosen
        .into_iter()
        .map(|(i, _)| i)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect();
    indices.sort_by_key(|i| (records[*i].cursor, *i));
    // Return the newest selected context within the byte budget, in chronological order.
    let mut used = 0;
    let mut output = Vec::new();
    let max = args.max_chars.unwrap_or(6000);
    let mut text_truncated = false;
    for &i in indices.iter().rev() {
        // Bound metadata as well as text: many empty context lines are not free.
        if output.len() == 400 {
            text_truncated = true;
            break;
        }
        let line = &records[i];
        let remaining = max - used;
        if remaining == 0 {
            text_truncated = true;
            break;
        }
        let mut start = line.text.len().saturating_sub(remaining);
        while !line.text.is_char_boundary(start) {
            start += 1;
        }
        let text = &line.text[start..];
        used += text.len();
        text_truncated |= start > 0;
        output.push(
            json!({"stream":line.stream,"text":text,"cursor":line.cursor,
            "matched":matched.contains(&i),"partial":line.partial,"textTruncated":start>0}),
        );
    }
    output.reverse();
    let matches_omitted =
        count.saturating_sub(output.iter().filter(|line| line["matched"] == true).count());
    json!({"lines":output,"matchedLines":count,"matchesOmitted":matches_omitted,
        "truncated":text_truncated || matches_omitted>0,"bytesReturned":used,"maxReturnedLines":400})
}

impl TaskManager {
    pub fn observe(&self, args: &ObserveArgs) -> Result<Value, Failure> {
        let regex = settings(args)?;
        let (events, cursor, oldest, status, limited, persistence) = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| ("INTERNAL", "Task registry unavailable"))?;
            let task = state
                .tasks
                .iter()
                .find(|t| t.id == args.task_id)
                .ok_or(("NOT_FOUND", "Job not found or no longer retained"))?;
            if args.cursor.is_some_and(|c| c > state.sequence) {
                return Err(("ARGUMENT", "cursor exceeds latest retained sequence"));
            }
            let events = state
                .journal
                .iter()
                .filter(|e| e["taskId"] == args.task_id)
                .cloned()
                .collect::<Vec<_>>();
            let oldest = state
                .journal
                .front()
                .and_then(|e| e["cursor"].as_u64())
                .unwrap_or(state.sequence.saturating_add(1));
            (
                events,
                state.sequence,
                oldest,
                task.status,
                task.result.as_ref().is_some_and(|r| r["limited"] == true),
                persistence_error(&state),
            )
        };
        let mut result = select(&events, args, regex.as_ref());
        result["taskId"] = json!(args.task_id);
        result["status"] = json!(status);
        result["cursor"] = json!(cursor);
        result["oldestCursor"] = json!(oldest);
        result["gap"] = json!(args.cursor.unwrap_or(0).saturating_add(1) < oldest);
        result["outputLimited"] = json!(limited);
        result["absenceProven"] = json!(false);
        result["textLossy"] = json!(events.iter().any(|e| e["data"]["textLossy"] == true));
        result["persistenceError"] = persistence;
        result["coverage"] = json!(
            "retained-job-events-only; context is limited to the cursor window; leading lines may be partial after eviction; no arbitrary server commands or log files"
        );
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn args() -> ObserveArgs {
        serde_json::from_value(
            json!({"taskId":"job","filter":"ERROR|panic","lines":1,"before":0,"after":2}),
        )
        .unwrap()
    }
    #[test]
    fn reconstructs_split_matches_and_keeps_stack_context_per_stream() {
        let events = vec![
            json!({"kind":"output","cursor":1,"data":{"stream":"stderr","text":"ER"}}),
            json!({"kind":"output","cursor":2,"data":{"stream":"stdout","text":"noise\n"}}),
            json!({"kind":"output","cursor":3,"data":{"stream":"stderr","text":"ROR failure\n frame1\n frame2\n"}}),
        ];
        let mut a = args();
        let regex = settings(&a).unwrap();
        let r = select(&events, &a, regex.as_ref());
        assert_eq!(r["matchedLines"], 1);
        assert_eq!(r["lines"].as_array().unwrap().len(), 3);
        assert_eq!(r["lines"][0]["text"], "ERROR failure\n");
        assert_eq!(r["lines"][2]["text"], " frame2\n");
        a.cursor = Some(1);
        let next = select(&events, &a, regex.as_ref());
        assert_eq!(next["lines"][0]["text"], "ERROR failure\n");
    }
    #[test]
    fn validates_regex_and_bounds_unicode_output() {
        let mut a = args();
        a.filter = Some("[".into());
        assert!(settings(&a).is_err());
        a.filter = None;
        a.max_chars = Some(256);
        let events = vec![
            json!({"kind":"output","cursor":1,"data":{"stream":"stderr","text":"é".repeat(1000)}}),
        ];
        let r = select(&events, &a, None);
        assert_eq!(r["bytesReturned"], 256);
        assert_eq!(r["truncated"], true);
    }
}
