# Manager approval chain

Expenses know their `submitter` and direct `approver`; higher management
reaches the approver through downward `manages` edges — skip-level approval
without per-report grants. `no_self_approval` shows relationship-based
DENY rules (the submitter may hold other grants, but never self-approves).

Try: `reaper-cli library run rebac/manager-approval`
