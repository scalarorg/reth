#!/bin/bash

# Define variables
DATA_DIR="./scripts/tmp/screth"
GENESIS_FILE="./scripts/genesis.json"

# Remove existing data directory
rm -rf $DATA_DIR

# Set the RUST_LOG environment variable for logging
export RUST_LOG="info"

# Run the Reth node with the specified parameters
# cargo run --release -- node --chain $GENESIS_FILE \
#     --datadir $DATA_DIR --auto-mine --http 

cargo run -- node --dev.block-time 30s --txpool.pending-max-count 500000 --txpool.queued-max-count 500000 --txpool.pending-max-size 100 --chain $GENESIS_FILE \
    --datadir $DATA_DIR --auto-mine --http 
