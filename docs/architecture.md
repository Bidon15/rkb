# Architecture

## Overview

The sequencer orchestrates three main components:

1. **Consensus** - PoA block production using commonware primitives
2. **Execution** - Block execution via reth Engine API
3. **Celestia** - Data availability and finality tracking

## Data Flow

```
Transactions → Consensus → Execution → Celestia
                  ↓            ↓           ↓
              Block Prod   Soft Conf   Firm Conf
```

## Finality Model

- **Soft Confirmation**: Immediate on PoA consensus
- **Firm Confirmation**: ~12s on Celestia finality

Maps to Ethereum forkchoice:
- `HEAD` = Latest soft-confirmed block
- `SAFE` = Same as HEAD (PoA is authoritative)
- `FINALIZED` = Latest Celestia-finalized block

## Components

See individual crate documentation for details.
