#!/bin/bash
# Generate keys for 3-validator PoA network
#
# This script generates:
# - Validator keys (Ed25519) for consensus signing
# - Celestia keys (secp256k1) for blob submission - ONE PER VALIDATOR
# - JWT secrets for reth Engine API
#
# IMPORTANT: Each validator needs its own funded Celestia account!

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEYS_DIR="$SCRIPT_DIR/../keys"

mkdir -p "$KEYS_DIR"

echo "============================================"
echo "Generating keys for 3-validator PoA network"
echo "============================================"
echo ""

echo "1. Generating Ed25519 validator keys..."
echo "   (Used for consensus message signing)"
echo ""

# For deterministic testing, we use the seed as the first 8 bytes of the key
# In production, use properly generated random keys!
#
# The key format is: 32 bytes hex (64 chars)
# First 8 bytes are read as u64 little-endian seed
# Remaining 24 bytes are ignored (padding)

# Helper function to generate little-endian hex seed
# Usage: le_seed_hex <seed_value>
le_seed_hex() {
    local seed=$1
    # Convert to 8-byte little-endian hex
    printf '%02x%02x%02x%02x%02x%02x%02x%02x' \
        $((seed & 0xFF)) \
        $(((seed >> 8) & 0xFF)) \
        $(((seed >> 16) & 0xFF)) \
        $(((seed >> 24) & 0xFF)) \
        $(((seed >> 32) & 0xFF)) \
        $(((seed >> 40) & 0xFF)) \
        $(((seed >> 48) & 0xFF)) \
        $(((seed >> 56) & 0xFF))
}

# Validator 0 (seed=0)
echo "$(le_seed_hex 0)000000000000000000000000000000000000000000000000" > "$KEYS_DIR/validator-0.key"
echo "   Generated validator-0.key (seed=0)"

# Validator 1 (seed=1)
echo "$(le_seed_hex 1)000000000000000000000000000000000000000000000000" > "$KEYS_DIR/validator-1.key"
echo "   Generated validator-1.key (seed=1)"

# Validator 2 (seed=2)
echo "$(le_seed_hex 2)000000000000000000000000000000000000000000000000" > "$KEYS_DIR/validator-2.key"
echo "   Generated validator-2.key (seed=2)"

echo ""
echo "2. Generating secp256k1 Celestia keys..."
echo "   (Used for PayForBlobs transactions - EACH NEEDS FUNDING!)"
echo ""

# Generate random Celestia keys (secp256k1 private keys, 32 bytes hex)
# Each validator needs its own funded account to submit blobs when it's the leader
openssl rand -hex 32 > "$KEYS_DIR/celestia-0.key"
echo "   Generated celestia-0.key (validator 0's Celestia account)"

openssl rand -hex 32 > "$KEYS_DIR/celestia-1.key"
echo "   Generated celestia-1.key (validator 1's Celestia account)"

openssl rand -hex 32 > "$KEYS_DIR/celestia-2.key"
echo "   Generated celestia-2.key (validator 2's Celestia account)"

echo ""
echo "3. Generating JWT secrets for reth Engine API..."
echo ""

# Generate random JWT secrets (32 bytes hex = 64 chars)
openssl rand -hex 32 > "$KEYS_DIR/jwt-0.hex"
openssl rand -hex 32 > "$KEYS_DIR/jwt-1.hex"
openssl rand -hex 32 > "$KEYS_DIR/jwt-2.hex"
echo "   Generated jwt-0.hex, jwt-1.hex, jwt-2.hex"

echo ""
echo "============================================"
echo "All keys generated in $KEYS_DIR"
echo "============================================"
echo ""

# Check if the sequencer binary exists to derive addresses
SEQUENCER_BIN="$SCRIPT_DIR/../../target/release/sequencer"
if [ -x "$SEQUENCER_BIN" ]; then
    echo "4. Deriving Celestia addresses..."
    echo ""
    ADDR_0=$("$SEQUENCER_BIN" celestia-address --key "$KEYS_DIR/celestia-0.key")
    ADDR_1=$("$SEQUENCER_BIN" celestia-address --key "$KEYS_DIR/celestia-1.key")
    ADDR_2=$("$SEQUENCER_BIN" celestia-address --key "$KEYS_DIR/celestia-2.key")
    echo "   Validator 0: $ADDR_0"
    echo "   Validator 1: $ADDR_1"
    echo "   Validator 2: $ADDR_2"
    echo ""
    echo "============================================"
    echo "FUND THESE ADDRESSES ON MOCHA TESTNET"
    echo "============================================"
    echo ""
    echo "Faucet: https://faucet.celestia-mocha.com/"
    echo ""
    echo "  $ADDR_0"
    echo "  $ADDR_1"
    echo "  $ADDR_2"
    echo ""
else
    echo "IMPORTANT: Fund ALL THREE Celestia accounts!"
    echo ""
    echo "Each validator submits blobs when it's the leader."
    echo "Without funding, blob submission will fail."
    echo ""
    echo "To get addresses, first build the sequencer:"
    echo "  cargo build --release"
    echo ""
    echo "Then run:"
    echo "  ./target/release/sequencer celestia-address --key docker/keys/celestia-0.key"
    echo "  ./target/release/sequencer celestia-address --key docker/keys/celestia-1.key"
    echo "  ./target/release/sequencer celestia-address --key docker/keys/celestia-2.key"
    echo ""
    echo "Fund these addresses via Mocha faucet:"
    echo "  https://faucet.celestia-mocha.com/"
    echo ""
fi

echo "To start the network:"
echo "  cd docker && docker compose up -d"
