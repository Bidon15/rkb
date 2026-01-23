# Task 04: Smart Contracts

## Owner
Agent Session 4

## Goal
Implement genesis contracts for cold start (IOU Registry) and prepare Hyperlane deployment.

## Deliverable
- `contracts/` directory with Solidity contracts
- Genesis allocation JSON for pre-deployed contracts
- Deployment scripts

---

## Dependencies

```toml
# For genesis generation (Rust side)
alloy-primitives = "0.8"
alloy-sol-types = "0.8"

# For contract development
# Use Foundry (forge) separately
```

---

## Contracts

### 1. IOURegistry

```solidity
// contracts/src/IOURegistry.sol
// SPDX-License-Identifier: MIT
pragma solidity ^0.8.20;

contract IOURegistry {
    mapping(address => uint256) public iouBalance;
    uint256 public totalIOUs;

    event IOUBurned(address indexed account, uint256 amount);

    // Note: Initial balances set via genesis storage slots
    // No constructor needed for genesis deployment

    function burn(uint256 amount) external {
        require(iouBalance[msg.sender] >= amount, "Insufficient IOU balance");

        iouBalance[msg.sender] -= amount;
        totalIOUs -= amount;

        // Burn native tokens by sending to zero address
        (bool success,) = address(0).call{value: amount}("");
        require(success, "Native token burn failed");

        emit IOUBurned(msg.sender, amount);
    }

    function getOutstandingIOUs() external view returns (uint256) {
        return totalIOUs;
    }

    function getIOUBalance(address account) external view returns (uint256) {
        return iouBalance[account];
    }
}
```

### 2. Hyperlane Contracts (Pre-deployed)

Pre-deploy standard Hyperlane contracts at genesis:
- Mailbox
- ISM (Interchain Security Module) - configured for your validators
- Warp Route (USDC)

```solidity
// These are standard Hyperlane contracts
// We just need to configure and pre-deploy them

// Addresses (deterministic, same across all deployments):
// Mailbox:    0x0000000000000000000000000000000000001000
// ISM:        0x0000000000000000000000000000000000001001
// WarpRoute:  0x0000000000000000000000000000000000001002
// IOURegistry: 0x0000000000000000000000000000000000001003
```

---

## Tasks

### 1. Project Setup
- [ ] Create `contracts/` directory
- [ ] Setup Foundry project
- [ ] Add dependencies

### 2. IOU Registry
- [ ] Implement IOURegistry.sol
- [ ] Write unit tests
- [ ] Generate bytecode

### 3. Genesis Storage Computation
- [ ] Compute storage slots for initial IOU balances
- [ ] Script to generate genesis allocation JSON

### 4. Hyperlane Preparation
- [ ] Download Hyperlane contract bytecode
- [ ] Configure ISM for validator set
- [ ] Compute initial storage state
- [ ] Document Ethereum-side setup

### 5. Genesis Generation Tool
- [ ] Rust tool to generate complete genesis.json
- [ ] Include: allocations + contract deployments
- [ ] Validate genesis correctness

---

## Genesis Allocation Format

```json
{
  "alloc": {
    "0x0000000000000000000000000000000000001003": {
      "code": "0x608060...",
      "storage": {
        "0x0": "300000000000000000000000",
        "0x1234...": "100000000000000000000000",
        "0x5678...": "100000000000000000000000",
        "0x9abc...": "100000000000000000000000"
      },
      "balance": "0x0"
    },
    "0xValidatorA": {
      "balance": "100000000000000000000000"
    },
    "0xValidatorB": {
      "balance": "100000000000000000000000"
    },
    "0xValidatorC": {
      "balance": "100000000000000000000000"
    }
  }
}
```

---

## Storage Slot Computation

```rust
// Compute storage slot for mapping(address => uint256)
fn compute_mapping_slot(mapping_slot: U256, key: Address) -> B256 {
    let mut hasher = Keccak256::new();
    hasher.update(key.as_slice());
    hasher.update(mapping_slot.to_be_bytes::<32>());
    B256::from_slice(&hasher.finalize())
}

// Example: iouBalance is at slot 0
// iouBalance[0xValidator] is at keccak256(validator || 0)
```

---

## Genesis Generation Tool

```rust
// src/genesis/mod.rs

pub struct GenesisConfig {
    pub chain_id: u64,
    pub validators: Vec<ValidatorConfig>,
    pub iou_amounts: HashMap<Address, U256>,
    pub hyperlane_config: HyperlaneConfig,
}

pub struct ValidatorConfig {
    pub address: Address,
    pub initial_balance: U256,  // IOU amount
}

impl GenesisConfig {
    pub fn generate(&self) -> Genesis {
        let mut alloc = BTreeMap::new();

        // Add validator balances (IOUs)
        for validator in &self.validators {
            alloc.insert(validator.address, GenesisAccount {
                balance: validator.initial_balance,
                ..Default::default()
            });
        }

        // Add IOURegistry contract
        alloc.insert(IOU_REGISTRY_ADDRESS, GenesisAccount {
            code: IOU_REGISTRY_BYTECODE.into(),
            storage: self.compute_iou_storage(),
            ..Default::default()
        });

        // Add Hyperlane contracts
        self.add_hyperlane_contracts(&mut alloc);

        Genesis {
            config: self.chain_config(),
            alloc,
            ..Default::default()
        }
    }
}
```

---

## Hyperlane Ethereum Side

Document what needs to happen on Ethereum:

```markdown
## Ethereum Setup (Manual)

1. Deploy Hyperlane Warp Route on Ethereum
   - Token: USDC (0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48)
   - Connected to: Your chain (chain ID: XXXXX)
   - ISM: MultisigISM with your validators

2. Configure Validators
   - Each validator runs Hyperlane relayer
   - Signs messages for your chain

3. Verify Connection
   - Test bridge with small amount
   - Confirm receipt on your chain
```

---

## Research Needed

1. **Hyperlane Contracts**
   - Latest version bytecode
   - ISM configuration for PoA
   - Warp Route setup

2. **Storage Layout**
   - Solidity storage slot computation
   - Mapping slot formula

3. **Genesis Format**
   - Reth genesis.json schema
   - Pre-deployed contract requirements

---

## Out of Scope

- Custom bridge (use Hyperlane)
- Governance contracts (future)
- Staking contracts (PoA doesn't need)

---

## Test Criteria

```bash
# Foundry tests
cd contracts && forge test

# Genesis generation
cargo test genesis::generate_valid_genesis

# Storage slot computation
cargo test genesis::storage_slot_computation
```

---

## Output

When complete:

1. `contracts/src/IOURegistry.sol` - Tested and auditable
2. `contracts/out/IOURegistry.json` - Compiled bytecode + ABI
3. `src/genesis/mod.rs` - Genesis generation tool
4. `docs/ethereum-setup.md` - Hyperlane Ethereum-side instructions
