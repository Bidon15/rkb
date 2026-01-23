#!/bin/bash
# Connect reth nodes for transaction gossip
#
# This script uses the admin_nodeInfo and admin_addPeer APIs to connect
# all reth nodes after startup. This is necessary because:
# 1. We run a private network (no public bootnodes)
# 2. Docker networking requires explicit peer connections
#
# The reth nodes form a SEPARATE p2p network from the consensus validators.
# - Consensus p2p (commonware): Block relay, consensus messages
# - Reth p2p (devp2p/eth): Transaction gossip
#
# Usage: ./connect-reth-peers.sh
#        (Run after 'docker compose up')

set -e

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

# RPC endpoints (inside docker network, use container names)
RETH_0="http://localhost:8545"
RETH_1="http://localhost:8546"
RETH_2="http://localhost:8547"

echo "============================================"
echo "Connecting reth nodes for tx gossip"
echo "============================================"
echo ""

# Function to get enode URL from a reth node
get_enode() {
    local rpc_url=$1
    local container_name=$2

    # Get node info
    local result=$(curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"admin_nodeInfo","params":[],"id":1}')

    # Extract enode and replace IP with container name for docker networking
    local enode=$(echo "$result" | grep -o '"enode":"[^"]*"' | sed 's/"enode":"//;s/"$//')

    if [ -z "$enode" ]; then
        echo "ERROR: Could not get enode from $rpc_url"
        echo "Response: $result"
        return 1
    fi

    # Replace 127.0.0.1 or any IP with container name for docker DNS
    # enode format: enode://<pubkey>@<ip>:<port>
    local pubkey=$(echo "$enode" | sed 's|enode://||;s|@.*||')
    echo "enode://${pubkey}@${container_name}:30303"
}

# Function to add a peer
add_peer() {
    local rpc_url=$1
    local enode=$2
    local from_name=$3
    local to_name=$4

    local result=$(curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d "{\"jsonrpc\":\"2.0\",\"method\":\"admin_addPeer\",\"params\":[\"$enode\"],\"id\":1}")

    local success=$(echo "$result" | grep -o '"result":true')
    if [ -n "$success" ]; then
        echo "  $from_name -> $to_name: Connected"
    else
        echo "  $from_name -> $to_name: Failed (may already be connected)"
        echo "  Response: $result"
    fi
}

# Function to get peer count
get_peer_count() {
    local rpc_url=$1
    local result=$(curl -s -X POST "$rpc_url" \
        -H "Content-Type: application/json" \
        -d '{"jsonrpc":"2.0","method":"net_peerCount","params":[],"id":1}')

    local count_hex=$(echo "$result" | grep -o '"result":"0x[^"]*"' | sed 's/"result":"//;s/"$//')
    echo $((count_hex))
}

echo "1. Getting enode URLs..."
echo ""

ENODE_0=$(get_enode "$RETH_0" "reth-0") || exit 1
echo "   reth-0: ${ENODE_0:0:80}..."

ENODE_1=$(get_enode "$RETH_1" "reth-1") || exit 1
echo "   reth-1: ${ENODE_1:0:80}..."

ENODE_2=$(get_enode "$RETH_2" "reth-2") || exit 1
echo "   reth-2: ${ENODE_2:0:80}..."

echo ""
echo "2. Connecting peers..."
echo ""

# Connect each node to the others (mesh topology)
# reth-0 connects to reth-1 and reth-2
add_peer "$RETH_0" "$ENODE_1" "reth-0" "reth-1"
add_peer "$RETH_0" "$ENODE_2" "reth-0" "reth-2"

# reth-1 connects to reth-0 and reth-2
add_peer "$RETH_1" "$ENODE_0" "reth-1" "reth-0"
add_peer "$RETH_1" "$ENODE_2" "reth-1" "reth-2"

# reth-2 connects to reth-0 and reth-1
add_peer "$RETH_2" "$ENODE_0" "reth-2" "reth-0"
add_peer "$RETH_2" "$ENODE_1" "reth-2" "reth-1"

echo ""
echo "3. Verifying connections..."
echo ""

# Wait a moment for connections to establish
sleep 2

PEERS_0=$(get_peer_count "$RETH_0")
PEERS_1=$(get_peer_count "$RETH_1")
PEERS_2=$(get_peer_count "$RETH_2")

echo "   reth-0: $PEERS_0 peers"
echo "   reth-1: $PEERS_1 peers"
echo "   reth-2: $PEERS_2 peers"

echo ""
if [ "$PEERS_0" -ge 2 ] && [ "$PEERS_1" -ge 2 ] && [ "$PEERS_2" -ge 2 ]; then
    echo "============================================"
    echo "SUCCESS: All reth nodes connected!"
    echo "============================================"
    echo ""
    echo "Transaction gossip is now enabled."
    echo "Transactions sent to any reth node will propagate to all others."
else
    echo "============================================"
    echo "WARNING: Not all nodes fully connected"
    echo "============================================"
    echo ""
    echo "Some connections may still be establishing."
    echo "Run this script again or check docker logs."
fi
