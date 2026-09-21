# 0003. Queue ordering, and where lane defaults come from

Status: accepted (M0) · Needs sign-off: Dev A

## Context

§6.2 says `Scheduled` is "the default source of truth for ordering" and a manual task
"joins the lane at `Normal` and takes its normal turn". §6.1 lists default lanes named
`workspaces`, `fetchers`, `notify`, `index`, while §12 rule 7 forbids the queue hardcoding
module names.

## Decision

* Within a lane: FIFO by arrival, with `Overridden` tasks ahead of all non-overridden ones
  (and behind earlier overrides). `Scheduled` and `Normal` share arrival order; neither
  outranks the other. Priority promotion and reordering themselves are M2.
* The queue knows exactly one lane, `default` (4). Every other lane exists because a module
  declared it in `lanes()`, so the §6.1 table is the contract each module implements, not a
  list the daemon owns. `config.toml` `[lanes]` overrides `max_concurrent` for declared lanes.
* Two modules declaring the same lane with different `max_concurrent` is a startup failure.
* A module's `init` failure marks it unavailable (ops answer `unavailable`); it does not stop
  the daemon.
