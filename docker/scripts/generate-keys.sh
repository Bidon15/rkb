#!/bin/bash
# Generate keys for 3-validator PoA network
#
# This script generates:
# - Validator keys (Ed25519) for each validator
# - JWT secrets for reth Engine API
# - Celestia signing key (placeholder - replace with real key)

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
KEYS_DIR="$SCRIPT_DIR/../keys"

mkdir -p "$KEYS_DIR"

echo "Generating validator keys..."

# For deterministic testing, we use the seed as the first 8 bytes of the key
# In production, use properly generated random keys!

# Validator 0 (seed=0)
printf '%016x%048x' 0 0 > "$KEYS_DIR/validator-0.key"
echo "Generated validator-0.key (seed=0)"

# Validator 1 (seed=1)
printf '%016x%048x' 1 0 > "$KEYS_DIR/validator-1.key"
echo "Generated validator-1.key (seed=1)"

# Validator 2 (seed=2)
printf '%016x%048x' 2 0 > "$KEYS_DIR/validator-2.key"
echo "Generated validator-2.key (seed=2)"

echo ""
echo "Generating JWT secrets for reth..."

# Generate random JWT secrets (32 bytes hex = 64 chars)
openssl rand -hex 32 > "$KEYS_DIR/jwt-0.hex"
openssl rand -hex 32 > "$KEYS_DIR/jwt-1.hex"
openssl rand -hex 32 > "$KEYS_DIR/jwt-2.hex"
echo "Generated jwt-0.hex, jwt-1.hex, jwt-2.hex"

echo ""
echo "Creating placeholder Celestia key..."
# In production, this should be a real secp256k1 key funded on Celestia
# For testing, create a placeholder
openssl rand -hex 32 > "$KEYS_DIR/celestia.key"
echo "Generated celestia.key (placeholder - fund this address on Mocha testnet!)"

echo ""
echo "All keys generated in $KEYS_DIR"
echo ""
echo "IMPORTANT: For Celestia Mocha testnet:"
echo "  1. Import celestia.key into a wallet"
echo "  2. Get the address and fund it via faucet: https://faucet.celestia-mocha.com/"
echo ""
echo "To start the network:"
echo "  cd docker && docker compose up -d"
