#!/bin/bash
DIR="$( cd "$( dirname "$0" )" && pwd )"
ENV=${ENV:-local}
export BLOCKSCOUT_SUBNET=${SUBNET_PREFIX:-172.16.20}
export BLOCKSCOUT_RUNTIME=${DOCKER_RUNTIME:-${DIR}/.runtime}/blockscout

start() {
    echo "Starting Blockscout with env ${ENV} and runtime dir ${BLOCKSCOUT_RUNTIME}"
    rm -rf ${BLOCKSCOUT_RUNTIME}
    mkdir -p ${BLOCKSCOUT_RUNTIME}
    chmod 777 ${BLOCKSCOUT_RUNTIME}
    docker compose -f ${DIR}/docker-compose.yml up -d
}

stop() {
    docker compose -f ${DIR}/docker-compose.yml down
}
$@