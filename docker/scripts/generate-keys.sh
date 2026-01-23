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

# Validator 0 (seed=0)
printf '%016x%048x' 0 0 > "$KEYS_DIR/validator-0.key"
echo "   Generated validator-0.key (seed=0)"

# Validator 1 (seed=1)
printf '%016x%048x' 1 0 > "$KEYS_DIR/validator-1.key"
echo "   Generated validator-1.key (seed=1)"

# Validator 2 (seed=2)
printf '%016x%048x' 2 0 > "$KEYS_DIR/validator-2.key"
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
echo "IMPORTANT: Fund ALL THREE Celestia accounts!"
echo ""
echo "Each validator submits blobs when it's the leader."
echo "Without funding, blob submission will fail."
echo ""
echo "Steps:"
echo "  1. Get addresses from each celestia-*.key"
echo "     (Import into a wallet or use celestia-node CLI)"
echo ""
echo "  2. Fund each address via Mocha faucet:"
echo "     https://faucet.celestia-mocha.com/"
echo ""
echo "  3. Verify balances before starting the network"
echo ""
echo "To start the network:"
echo "  cd docker && docker compose up -d"
