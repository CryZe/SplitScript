# Drug Dealer Simulator autosplitter port

This maintained port is based on `LiveSplit.DDS.asl` from the reviewed ASL
corpus. It motivates compile-time settings families: the original creates one
boolean setting for every level from 2 through 36 in its `startup` loop.

## Preserved behavior

| ASL behavior | SplitScript representation |
| --- | --- |
| Attach to ASL's extensionless process name | `state "DrugDealerSimulator-Win64-Shipping.exe"` for the current Windows host contract |
| Read the six-step 64-bit level pointer | The same `at` pointer path |
| Register settings `2` through `36` | `for level in 2..=36` inside `settings` |
| Use each decimal level as key and label | `` `{level}` key `{level}` `` |
| Enable every generated setting by default | One `true` family default |
| Split only when the level increases | The original edge predicate |
| Select the split setting dynamically | `settings.enabled(current.level as String)` |

The family is compile-time syntax, not mutable runtime settings creation. It
lowers to the same ordinary boolean declarations used by hand-written settings,
so host registration, persistence, snapshots, tooltips, and dynamic lookup have
one implementation. Generated settings deliberately do not invent source
identifiers such as `level17`; data-driven code uses their stable string keys.

## Runtime status

The deterministic host fixture covers the exact `.exe` process name, the full
64-bit pointer chain, registration order and metadata for all 35 settings,
initial snapshot seeding, enabled and disabled level transitions, live settings
refresh, two splits, and detach.
