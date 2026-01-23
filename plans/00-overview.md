# Project Overview

## Parallel Work Streams

This project is split into independent work streams that can be developed in parallel.

```
┌─────────────────────────────────────────────────────────────────┐
│                         ARCHITECTURE                            │
│                                                                 │
│  ┌─────────────┐  ┌─────────────┐  ┌─────────────┐              │
│  │ 01-consensus│  │ 02-celestia │  │03-execution │              │
│  │             │  │             │  │             │              │
│  │ commonware  │  │   Lumina    │  │ Engine API  │              │
│  │ PoA         │  │   client    │  │ to reth     │              │
│  └──────┬──────┘  └──────┬──────┘  └──────┬──────┘              │
│         │                │                │                     │
│         └────────────────┼────────────────┘                     │
│                          │                                      │
│                          ▼                                      │
│                 ┌─────────────────┐                             │
│                 │  04-contracts   │                             │
│                 │                 │                             │
│                 │  IOU Registry   │                             │
│                 │  Genesis setup  │                             │
│                 └─────────────────┘                             │
│                          │                                      │
│                          ▼                                      │
│                 ┌─────────────────┐                             │
│                 │ 05-integration  │                             │
│                 │                 │                             │
│                 │ Wire it all     │                             │
│                 │ together        │                             │
│                 └─────────────────┘                             │
└─────────────────────────────────────────────────────────────────┘
```

## Work Streams

| File | Stream | Can Parallelize | Dependencies |
|------|--------|-----------------|--------------|
| `01-consensus.md` | PoA Consensus | ✓ Yes | None |
| `02-celestia.md` | Celestia/Lumina | ✓ Yes | None |
| `03-execution.md` | Engine API | ✓ Yes | None |
| `04-contracts.md` | Smart Contracts | ✓ Yes | None |
| `05-integration.md` | Wire Together | After 01-04 | 01, 02, 03, 04 |
| `06-infrastructure.md` | Docker/Deploy | ✓ Yes | None |

## Parallel Execution

```
Week 1-2:
  ├── Agent 1: 01-consensus.md
  ├── Agent 2: 02-celestia.md
  ├── Agent 3: 03-execution.md
  └── Agent 4: 04-contracts.md + 06-infrastructure.md

Week 3-4:
  └── All agents: 05-integration.md (merge work)
```

## Shared Types

All work streams share types defined in `src/types/`. Coordinate on:
- Block structure
- Transaction format
- Config structures

Define these first or have one agent own `src/types/` and others import.
