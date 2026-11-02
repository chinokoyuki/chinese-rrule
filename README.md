# chinese-rrule

[![crates.io](https://img.shields.io/crates/v/chinese-rrule.svg)](https://crates.io/crates/chinese-rrule)
[![docs.rs](https://docs.rs/chinese-rrule/badge.svg)](https://docs.rs/chinese-rrule)
[![CI](https://github.com/chinokoyuki/chinese-rrule/actions/workflows/ci.yml/badge.svg)](https://github.com/chinokoyuki/chinese-rrule/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[中文说明](README_ZH.md)

Parse Chinese natural-language time expressions into RFC 5545 RRULE strings and next-trigger timestamps.

`每天早上8点提醒我喝水` → `FREQ=DAILY;BYHOUR=8;BYMINUTE=0`. That's the whole idea.

One dependency (chrono), no async, no LLM required. Pure functions, 30 tests, sub-microsecond per parse.

## Where this fits

- Reminder bots or calendar integrations taking free-form Chinese input
- Chat tools that need to detect "this message contains a schedule" without a round-trip to an LLM
- Anything already consuming RRULE (ics / cal-rrule / full-calendar backends)

## Install

```toml
[dependencies]
chinese-rrule = "0.1"
```

## Usage

```rust
use chinese_rrule::parse_time;

let r = parse_time("每周三下午3点站会").unwrap();

assert_eq!(r.rrule, "FREQ=WEEKLY;BYDAY=WE;BYHOUR=15;BYMINUTE=0");
assert!(!r.one_shot);
assert_eq!(r.confidence, 1.0); // explicit time found
assert!(r.next_trigger_at > 0); // epoch seconds, next Wednesday 15:00 local
```

Full runnable version: [`examples/quickstart.rs`](examples/quickstart.rs) (`cargo run --example quickstart`):

```
每天早上8点提醒我喝水
  → [recurring] rrule="FREQ=DAILY;BYHOUR=8;BYMINUTE=0" next=2026-11-03 08:00 (confidence 1.0)
每周三下午3点站会
  → [recurring] rrule="FREQ=WEEKLY;BYDAY=WE;BYHOUR=15;BYMINUTE=0" next=2026-11-04 15:00 (confidence 1.0)
每月1号还款
  → [recurring] rrule="FREQ=MONTHLY;BYMONTHDAY=1;BYHOUR=9;BYMINUTE=0" next=2026-12-01 09:00 (confidence 0.5)
工作日每天早上9点
  → [recurring] rrule="FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9;BYMINUTE=0" next=2026-11-03 09:00 (confidence 1.0)
明天下午2点提醒我拿快递
  → [one-shot] rrule="—" next=2026-11-03 14:00 (confidence 1.0)
月底提醒我还信用卡
  → [one-shot] rrule="—" next=2026-11-30 09:00 (confidence 0.5)
```

## Supported patterns

**Recurring** → RRULE string:

| Input | Output |
|---|---|
| 每天早上8点 | `FREQ=DAILY;BYHOUR=8` |
| 每周三下午3点 | `FREQ=WEEKLY;BYDAY=WE;BYHOUR=15` |
| 每月1号 | `FREQ=MONTHLY;BYMONTHDAY=1` |
| 工作日每天早上9点 | `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9` |
| 隔天早上8点 | `FREQ=DAILY;INTERVAL=2;BYHOUR=8` |

**One-shot** → empty RRULE, `next_trigger_at` timestamp, `one_shot = true`:

明天 / 后天 / 大后天, 下周X, 这周X, 本周X, N天后 (Arabic or Chinese numerals: 3天 / 三天 / 三十天), 月底, 下月X号, bare 周X (nearest occurrence).

**Time extraction**: `8点`, `8:30`, `8：30`, `八点`, `十二点`, `3点半`, `3点20`, plus 早上的/下午/晚上/傍晚/凌晨/深夜 period adjustment.

**Confidence**: `1.0` — pattern and explicit time both found; `0.5` — pattern found, time defaulted to 9:00; `0.8` — from the LLM fallback below.

## LLM fallback (bring your own)

When `parse_time` returns `None`, complex expressions ("每逢节气提醒我", "我妈生日前两天") are out of scope for rules. If you have an LLM handy:

```rust
use chinese_rrule::{parse_time, parse_llm_time_json, LLM_TIME_PARSE_PROMPT};

match parse_time(msg) {
    Some(r) => { /* fast path */ }
    None => {
        // 1. Send LLM_TIME_PARSE_PROMPT to any chat model,
        //    with the user message + current Unix timestamp in the user prompt.
        // 2. Parse the JSON response:
        if let Some(r) = parse_llm_time_json(&llm_response) {
            // r.confidence == 0.8
        }
    }
}
```

The crate itself stays offline — the prompt and response parser are provided, the model is yours.

## Notes

- All timestamps are local-time; RRULE strings embed `BYHOUR`/`BYMINUTE` in local time too.
- The crate produces RRULE strings; it does not expand them into occurrences. Pair it with a scheduler on your side.
- No `serde` — the result struct is four plain fields, map it however you like.

## License

MIT
