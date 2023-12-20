# Reth test cluster

## Description
This crate helps run test cases base on config from .env. The test cases are:
- `send_raw_transaction_test.rs`: Send X raw transactions to reth cluster

## Progress
- Done: 
    - Run X reth nodes with config from .env at workspace
    - Build and run on Docker: config file .env to docker and make sure reth-test-cluster can read environment variable through `dotenvy` 
    - Check shutdown when clusters run all tests
- TODO:
    - Check connectivity between clusters
    - Push transactions to each node