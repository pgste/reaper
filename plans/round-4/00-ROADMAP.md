# Round 4 — Competitive Parity & Language Growth

Round 3 closed the GA-hardening ledger (plans/round-3/06, phases A–F all
shipped or decision-recorded). Round 4 is forward-looking: it turns the
comparative analyses written at the end of round 3 into buildable plans.

| Plan | Source analysis | Theme |
|---|---|---|
| `01-dsl-parity-and-fast-path.md` | `docs/development/REGO_GAP_ANALYSIS.md` | Close the compiled/AST cliff; Rego authoring parity; stdlib gaps |
| (unscheduled) list-authorization filter API | `docs/development/FILTER_COMPILATION.md` | Build when pulled by a consumer (G.1–G.4 phased there) |
| (parked) tier-2 specialization | `docs/development/PARTIAL_EVALUATION.md` §6.1 | Revisit triggers documented; do not schedule |

Ordering inside round 4: plan 01 phase A before anything else — it shrinks
the fast-path cliff with mechanical, differential-gated work and is a
precondition for the filter/pruning story ever covering the `input` policy
class.
