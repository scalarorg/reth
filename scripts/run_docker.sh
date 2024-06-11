docker run \
  -v rethdata:/root/.local/share/reth/devnet \
  -d \
  -p 9001:9001 \
  -p 8545:8545 \
  -p 30303:30303 \
  -p 30303:30303/udp \
  --name reth \
  reth:local \
  node \
  --dev \
  --dev.block-time 8sec \
  --metrics 0.0.0.0:9001 \
  --http --http.addr 0.0.0.0 --http.port 8545 \
