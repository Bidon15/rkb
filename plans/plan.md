# Celestia-Native PoA EVM Stack

## Vision

A minimal, production-ready stack that enables PoA L1 teams to deploy EVM chains with Celestia DA in two containers.

**One-liner:** "Keep your PoA validators. Get Celestia DA + finality. Two containers. Deploy in 5 minutes."

---

## Problem Statement

PoA L1 teams today face a choice:

| Option | Problem |
|--------|---------|
| Run their own DA | Storage grows forever, no external finality |
| Use existing rollup stacks | Over-engineered for single-chain PoA use case |
| Adopt Astria | Shared sequencer model, multi-rollup complexity |

**We solve this by:** Building the thinnest possible glue between three production-grade components (commonware consensus, Lumina/Celestia, vanilla reth).

---

## Target Customers

- Teams currently running PoA L1s (BSC-style, Polygon PoS-style, enterprise chains)
- Teams building new PoA chains who want modern DA
- Projects that need external finality for bridging (Hyperlane, etc.)

**What they want:**
- Keep their validator set and operational model
- Offload block storage to Celestia
- External finality guarantee
- EVM compatibility with upstream updates
- Minimal operational overhead

---

## Architecture

```
┌───────────────────────────────────────────────────────────────┐
│  SEQUENCER BINARY                                             │
│                                                               │
│  ┌─────────────────┐  ┌─────────────────┐  ┌───────────────┐  │
│  │ commonware-     │  │ Lumina          │  │ Engine API    │  │
│  │ consensus       │  │ (embedded)      │  │ Client        │  │
│  │                 │  │                 │  │               │  │
│  │ PoA ordering    │  │ Celestia DA     │  │ → Reth        │  │
│  └────────┬────────┘  └────────┬────────┘  └───────┬───────┘  │
│           │                    │                   │          │
│           └────────────────────┼───────────────────┘          │
│                                │                              │
└────────────────────────────────┼──────────────────────────────┘
                                 │
                    ┌────────────┴────────────┐
                    │                         │
                    ▼                         ▼
            Celestia Network            Vanilla Reth
```

### Data Flow

```
1. User submits tx
         │
         ▼
2. PoA validators reach consensus (commonware-consensus)
         │
         ▼
3. SOFT CONFIRMATION (instant)
         │
         ├──────────────────────────────┐
         │                              │
         ▼                              ▼
4. Execute on Reth              5. Submit blob to Celestia
   (HEAD + SAFE)                   (via embedded Lumina)
                                        │
                                        ▼
                               6. Celestia finalizes (~12s)
                                        │
                                        ▼
                               7. FIRM CONFIRMATION
                                  (update FINALIZED on Reth)
```

### Finality Model

| Level | Source | Latency | Guarantee |
|-------|--------|---------|-----------|
| **Soft** | PoA validator signatures | ~100-500ms | Validators agreed |
| **Firm** | Celestia finality | ~12s | External DA, immutable |

**Reth forkchoice mapping:**
- Soft → `HEAD` + `SAFE`
- Firm → `FINALIZED`

---

## Components

### 1. Consensus Module (`src/consensus/`)

**Purpose:** Wrap commonware-consensus for PoA block production

**Responsibilities:**
- Manage validator set
- Leader election / rotation
- Transaction ordering
- Block signing and propagation

**Dependencies:**
- `commonware-consensus`
- `commonware-p2p`
- `commonware-cryptography`

**Estimated lines:** ~300

### 2. Celestia Module (`src/celestia/`)

**Purpose:** Submit blocks to Celestia, track finality

**Responsibilities:**
- Encode blocks as blobs
- Submit via embedded Lumina
- Subscribe to finality events
- Emit firm confirmations

**Dependencies:**
- `lumina-node`

**Estimated lines:** ~200

### 3. Execution Module (`src/execution/`)

**Purpose:** Drive reth via Engine API

**Responsibilities:**
- Build `ExecutionPayload` from consensus blocks
- Call `engine_newPayloadV3`
- Call `engine_forkchoiceUpdatedV3`
- Track soft/firm state

**Dependencies:**
- `jsonrpsee` (HTTP client)
- `alloy-rpc-types-engine`

**Estimated lines:** ~400

### 4. Types (`src/types/`)

**Purpose:** Core data structures

**Contents:**
- Block format
- Transaction wrapper
- Consensus messages
- Config structures

**Estimated lines:** ~200

### 5. CLI/Config (`src/main.rs`, `src/config.rs`)

**Purpose:** Binary entrypoint, configuration

**Config options:**
```toml
[consensus]
validators = ["0x123...", "0x456...", "0x789..."]
private_key_path = "/path/to/key"

[celestia]
network = "mainnet"  # or "mocha", "arabica"
namespace = "your_chain"

[execution]
reth_engine_url = "http://localhost:8551"
jwt_secret_path = "/path/to/jwt"
```

**Estimated lines:** ~200

---

## Total Codebase

| Module | Lines |
|--------|-------|
| Consensus | ~300 |
| Celestia | ~200 |
| Execution | ~400 |
| Types | ~200 |
| CLI/Config | ~200 |
| Tests | ~500 |
| **Total** | **~1800** |

---

## Dependencies

```toml
[dependencies]
# Consensus
commonware-consensus = "0.0.65"
commonware-p2p = "0.0.65"
commonware-cryptography = "0.0.65"
commonware-runtime = "0.0.65"

# Celestia (embedded light client)
lumina-node = "0.5"

# Engine API
jsonrpsee = { version = "0.24", features = ["http-client"] }
alloy-rpc-types-engine = "0.3"
alloy-primitives = "0.8"

# Runtime
tokio = { version = "1", features = ["full"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# Logging
tracing = "0.1"
tracing-subscriber = "0.3"
```

**No forks. All upstream crates.**

---

## Deployment

### Customer Deployment

```yaml
# docker-compose.yml
version: "3.8"

services:
  sequencer:
    image: your-product/sequencer:latest
    ports:
      - "26656:26656"  # P2P
      - "8080:8080"    # RPC (optional)
    environment:
      - VALIDATORS=0x123,0x456,0x789
      - CELESTIA_NETWORK=mainnet
      - RETH_ENGINE_URL=http://reth:8551
    volumes:
      - ./config:/config
      - sequencer-data:/data

  reth:
    image: ghcr.io/paradigmxyz/reth:latest
    command: >
      node
      --authrpc.addr=0.0.0.0
      --authrpc.port=8551
      --authrpc.jwtsecret=/config/jwt.hex
    ports:
      - "8545:8545"    # JSON-RPC
      - "8551:8551"    # Engine API
    volumes:
      - ./config:/config
      - reth-data:/data

volumes:
  sequencer-data:
  reth-data:
```

**Two containers. That's it.**

---

## Milestones

### M1: Core Loop (Week 1-2)

- [ ] Project scaffolding
- [ ] Basic consensus wrapper (static validator set)
- [ ] Engine API client (newPayload, forkchoiceUpdated)
- [ ] Hardcoded block production loop
- [ ] Integration test: sequencer → reth

**Deliverable:** Blocks flowing to reth without Celestia

### M2: Celestia Integration (Week 3)

- [ ] Lumina integration
- [ ] Blob encoding
- [ ] Submit on block finalization
- [ ] Finality subscription
- [ ] Soft/firm state machine

**Deliverable:** Full flow with Celestia finality

### M3: Networking (Week 4)

- [ ] commonware-p2p integration
- [ ] Transaction gossip
- [ ] Block propagation
- [ ] Multi-validator setup

**Deliverable:** Multi-node testnet

### M4: Hardening (Week 5-6)

- [ ] Configuration system
- [ ] CLI polish
- [ ] Error handling
- [ ] Logging/metrics
- [ ] Documentation
- [ ] Docker images

**Deliverable:** Production-ready binary

### M5: Testing & Launch (Week 7-8)

- [ ] Integration tests
- [ ] Testnet deployment (Celestia Mocha)
- [ ] Load testing
- [ ] Security review
- [ ] Mainnet deployment guide

**Deliverable:** Public release

---

## Future Considerations

### Not in V1, but potential future work:

| Feature | Complexity | Notes |
|---------|-----------|-------|
| Dynamic validator set | Medium | Add via governance tx |
| Slashing | Medium | For equivocation |
| PBS/MEV | High | Block builder separation |
| Based sequencing | High | Celestia-based leader election |
| ZK proving | High | For light client bridges |

### Bridging

V1 relies on external bridges (Hyperlane, LayerZero, etc.) that can verify against:
- PoA validator signatures (soft)
- Celestia finality proofs (firm)

Native bridge is out of scope - external bridges are more flexible and already production-ready.

---

## Non-Goals

Explicitly **not** building:

- Multi-rollup / shared sequencer (use Astria for that)
- Native bridge (use Hyperlane)
- Custom DA layer (use Celestia)
- Modified reth (stay vanilla)
- Complex fee markets (simple PoA fees)
- Decentralized sequencer set (PoA is the point)

---

## Success Metrics

| Metric | Target |
|--------|--------|
| Lines of code | < 2000 |
| Containers to deploy | 2 |
| Time to deploy | < 10 minutes |
| Soft confirmation latency | < 500ms |
| Firm confirmation latency | ~12s (Celestia bound) |
| Upstream dependency lag | 0 (no forks) |

---

## Native Gas Token

### Overview

The chain uses a custom native gas token (e.g., USDC or TIA) instead of ETH. This is **not** an ERC-20 - it's the protocol-level native token, just named differently.

```
┌─────────────────────────────────────────────────────────────────┐
│ NATIVE TOKEN ≠ ERC-20                                           │
│                                                                 │
│ ERC-20 USDC (on Ethereum):                                      │
│   - Contract at address 0xA0b8...                               │
│   - Requires separate ETH for gas                               │
│                                                                 │
│ Native USDC (on your chain):                                    │
│   - NO contract address                                         │
│   - IS the gas token                                            │
│   - Backed 1:1 by bridged USDC via Hyperlane                   │
└─────────────────────────────────────────────────────────────────┘
```

### Cold Start: IOU + Burn Mechanism

Genesis validators need native tokens to operate, but we want 1:1 backing. Solution: temporary IOUs that get burned once real USDC is bridged.

**Genesis State:**
```
Validator A: 100,000 native USDC (IOU)
Validator B: 100,000 native USDC (IOU)
Validator C: 100,000 native USDC (IOU)
IOURegistry: tracks who owes what

Hyperlane Mailbox: pre-deployed
Hyperlane Warp Route: pre-deployed, connected to Ethereum USDC
```

**IOU Registry Contract:**
```solidity
contract IOURegistry {
    mapping(address => uint256) public iouBalance;

    // Set via genesis storage slots
    // validatorA => 100_000e18
    // validatorB => 100_000e18
    // validatorC => 100_000e18

    function burn(uint256 amount) external {
        require(iouBalance[msg.sender] >= amount, "Insufficient IOU");
        iouBalance[msg.sender] -= amount;

        // Burn native tokens from sender
        (bool success,) = address(0).call{value: amount}("");
        require(success, "Burn failed");

        emit IOUBurned(msg.sender, amount);
    }

    function totalOutstandingIOUs() external view returns (uint256);
}
```

**Validator Cold Start Sequence:**

```
Step 1: Genesis
├── Validator has 100K native USDC (unbacked IOU)
├── Chain is operational
└── IOURegistry.iouBalance[validator] = 100K

Step 2: Validator bridges real USDC
├── Lock 100K USDC in Hyperlane on Ethereum
├── Hyperlane mints 100K native USDC on your chain
└── Validator now has 200K native (100K IOU + 100K backed)

Step 3: Validator burns IOU
├── Call IOURegistry.burn(100K)
├── Burns 100K native from validator balance
└── Validator now has 100K native (fully backed)

Step 4: Complete
├── IOURegistry.iouBalance[validator] = 0
├── Validator operates normally
└── 1:1 backing achieved
```

**Transparency:**
```
Total native supply = Hyperlane bridged supply + Outstanding IOUs

Anyone can verify:
- IOURegistry.totalOutstandingIOUs()
- Hyperlane Warp Route collateral on Ethereum

When all IOUs burned: Total supply == Bridged supply (fully backed)
```

### Steady State (Post Cold Start)

Once validators have burned their IOUs, the IOU mechanism is **never used again**.

**New users/validators simply:**
1. Bridge USDC from Ethereum via Hyperlane
2. Receive native USDC
3. Use for gas and transactions

No IOUs. No burn. Just normal bridging.

```
┌─────────────────────────────────────────────────────────────────┐
│ COLD START (one-time)           │ STEADY STATE (forever)        │
│─────────────────────────────────│───────────────────────────────│
│ Genesis validators only         │ All users                     │
│ IOU + Bridge + Burn             │ Just Bridge                   │
│ ~Day 1                          │ Day 2+                        │
└─────────────────────────────────────────────────────────────────┘
```

### Chain Metadata

```json
{
  "chainId": 12345,
  "chainName": "Your PoA Chain",
  "nativeCurrency": {
    "name": "USD Coin",
    "symbol": "USDC",
    "decimals": 18
  },
  "rpcUrls": ["https://rpc.yourchain.com"],
  "blockExplorerUrls": ["https://explorer.yourchain.com"]
}
```

---

## Open Questions

1. **Block format:** Use Ethereum-compatible block structure or custom? (Leaning Ethereum for tooling compatibility)

2. **Namespace strategy:** One Celestia namespace per chain? Per deployment?

3. **Genesis:** How to bootstrap? Genesis block in config or derived from Celestia?

4. **Validator key format:** Ed25519 (commonware native) or secp256k1 (Ethereum compatible)?

5. **RPC:** Expose sequencer RPC? Or just use reth's standard JSON-RPC?

6. **Native token:** USDC (Ethereum-bridged) or TIA (Celestia-native)?

---

## References

- [Commonware Library](https://github.com/commonwarexyz/monorepo)
- [Lumina (Celestia Rust client)](https://github.com/eigerco/lumina)
- [Reth](https://github.com/paradigmxyz/reth)
- [Engine API Spec](https://github.com/ethereum/execution-apis/tree/main/src/engine)
- [Celestia Documentation](https://docs.celestia.org/)
