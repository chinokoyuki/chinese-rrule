# chinese-rrule

[![crates.io](https://img.shields.io/crates/v/chinese-rrule.svg)](https://crates.io/crates/chinese-rrule)
[![docs.rs](https://docs.rs/chinese-rrule/badge.svg)](https://docs.rs/chinese-rrule)
[![CI](https://github.com/chinokoyuki/chinese-rrule/actions/workflows/ci.yml/badge.svg)](https://github.com/chinokoyuki/chinese-rrule/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md)

把中文自然语言时间表达式解析成 RFC 5545 RRULE 字符串和下次触发时间戳。

`每天早上8点提醒我喝水` → `FREQ=DAILY;BYHOUR=8;BYMINUTE=0`。就做这一件事。

单一依赖（chrono），无异步、不要求 LLM。纯函数，30 个测试，单次解析亚微秒。

## 适用场景

- 接收自由格式中文输入的提醒机器人、日历集成
- 需要在不请求 LLM 的情况下判断"这句话里有日程"的聊天工具
- 任何已经消费 RRULE 的系统（ics / cal-rrule / 各类日历后端）

## 安装

```toml
[dependencies]
chinese-rrule = "0.1"
```

## 用法

```rust
use chinese_rrule::parse_time;

let r = parse_time("每周三下午3点站会").unwrap();

assert_eq!(r.rrule, "FREQ=WEEKLY;BYDAY=WE;BYHOUR=15;BYMINUTE=0");
assert!(!r.one_shot);
assert_eq!(r.confidence, 1.0); // 找到了明确时间
assert!(r.next_trigger_at > 0); // epoch 秒，本地时区下周三 15:00
```

完整可运行版本见 [`examples/quickstart.rs`](examples/quickstart.rs)（`cargo run --example quickstart`）：

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

## 支持的模式

**重复规则** → RRULE 字符串：

| 输入 | 输出 |
|---|---|
| 每天早上8点 | `FREQ=DAILY;BYHOUR=8` |
| 每周三下午3点 | `FREQ=WEEKLY;BYDAY=WE;BYHOUR=15` |
| 每月1号 | `FREQ=MONTHLY;BYMONTHDAY=1` |
| 工作日每天早上9点 | `FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR;BYHOUR=9` |
| 隔天早上8点 | `FREQ=DAILY;INTERVAL=2;BYHOUR=8` |

**一次性** → 空 RRULE、`next_trigger_at` 时间戳、`one_shot = true`：

明天 / 后天 / 大后天、下周X、这周X、本周X、N天后（阿拉伯或中文数字：3天 / 三天 / 三十天）、月底、下月X号、裸周X（最近一次）。

**时间提取**：`8点`、`8:30`、`8：30`、`八点`、`十二点`、`3点半`、`3点20`，以及 早上/下午/晚上/傍晚/凌晨/深夜 的时段换算。

**置信度**：`1.0` — 模式和明确时间都找到；`0.5` — 找到模式，时间默认 9:00；`0.8` — 来自下面的 LLM 兜底。

## LLM 兜底（自带模型）

`parse_time` 返回 `None` 时，说明是规则覆盖不了的复杂表达（"每逢节气提醒我""我妈生日前两天"）。如果你手边有 LLM：

```rust
use chinese_rrule::{parse_time, parse_llm_time_json, LLM_TIME_PARSE_PROMPT};

match parse_time(msg) {
    Some(r) => { /* 快速路径 */ }
    None => {
        // 1. 把 LLM_TIME_PARSE_PROMPT 发给任意对话模型，
        //    用户消息里附上原句和当前 Unix 时间戳。
        // 2. 解析 JSON 响应：
        if let Some(r) = parse_llm_time_json(&llm_response) {
            // r.confidence == 0.8
        }
    }
}
```

本库自身保持离线——提示词和响应解析器是内置的，模型由你提供。

## 说明

- 所有时间戳按本地时区计算；RRULE 中的 `BYHOUR`/`BYMINUTE` 也是本地时间。
- 本库只生成 RRULE 字符串，不展开 occurrence；调度交给你的执行端。
- 不依赖 `serde`——结果结构就四个普通字段，随便映射。

## 许可

MIT
