#!/bin/bash
# Initialize a new sequencer deployment

set -e

echo "Initializing sequencer..."

# Generate config
./target/release/sequencer init --output config.toml

# Generate validator key
./target/release/sequencer keygen --output validator.key

echo "Initialization complete!"
echo "Edit config.toml and start with: ./target/release/sequencer run"
