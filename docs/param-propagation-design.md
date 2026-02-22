# Param Propagation to Dependencies

## Problem

Otto has no mechanism to pass parameter values from a parent task down to its
`before` dependencies. Values flow upward (dep -> dependent) via output/input
files, but not downward.

Example: `deploy` has an `account` param and depends on `build`, but can't tell
`build` which account to build for.

```yaml
deploy:
  params:
    account:
      default: home
      choices: [home, work]
  before: [build]
  bash: |
    cd ${account} && clasp push

build:
  bash: |
    node scripts/build.mjs  # no way to receive account from deploy
```

## How Make Solves This

GNU Make has **target-specific variables** (since mid-90s) which propagate down
to all prerequisites:

```makefile
deploy: account = work
deploy: lint test build push

build:
	node scripts/build.mjs $(account)

push:
	cd $(account) && clasp push
```

Running `make deploy` sets `account=work` for `deploy` and all its transitive
deps. It's dynamic scoping — the variable is set for the target and everything
it triggers.

## Current Otto Workarounds

The three existing mechanisms all fall short:

| Mechanism | Direction | Why it doesn't work |
|---|---|---|
| **pargs** | CLI -> named task | Only covers tasks named on CLI; deps get defaults |
| **output/input** | dep -> dependent (upward) | Opposite direction — deps run first |
| **task-level envs** | scoped to one task | Don't propagate to deps |

The practical workaround today is **inlining**: remove the dep from `before`
and call it directly in the task's script body.

## Proposed Design: Param Propagation by Name-Matching

When resolving params for a dependency task, if the parent has a resolved param
with the same name and the dep wasn't given an explicit value on the CLI,
inherit the parent's value.

### Implementation Scope

Localized to `parser.rs` around lines 700-770 where params are resolved.
Instead of only looking at `pargs` for the task, also check resolved values of
whichever task listed it as a `before` dep. Estimated ~50-100 lines of changes.

### The Diamond Problem

Otto deduplicates tasks in the graph — `build` appears as one node. If two
parents both depend on `build` with different values:

```yaml
deploy-staging:
  params: { account: { default: staging } }
  before: [build]

deploy-prod:
  params: { account: { default: prod } }
  before: [build]
```

Which `account` does `build` get? Make re-executes targets per invocation path
(no dedup), so it avoids this. Otto's graph scheduler deduplicates — `build`
runs once.

Three options:

| Approach | How | Tradeoff |
|---|---|---|
| **Conflict = error** | If two parents disagree on a propagated value, fail loudly | Safe, simple, covers 90% of cases |
| **Clone** | Create `build@deploy-staging` and `build@deploy-prod` as separate graph nodes | Clean but deps run multiple times |
| **First parent wins** | Nondeterministic, whoever resolves first | Bad idea, listed for completeness |

**Recommendation:** Start with "conflict = error". It's safe, simple, and
handles the common case (one parent pushing a value down). Clone behavior can
be added later as an opt-in if needed.
