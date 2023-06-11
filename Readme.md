# TickTock

## Build Docker image

```
docker build -t test -f ./Dockerfile .
```

## Setup

All type configuration are shown in `./demo-cfg.yaml`. Type `DictEntry` is used to get last block height of a subquery dictionary GraphQL endpoint. Type `ChainNode` is for a eth competitor RPC endpoint. Type `RedisKey` is used to monitor a redis key for last block height of a fetching service. Type "CFGMAPKey" is used for the old version fetching service. After that apply the ConfigMap in `./demo-cfg.yaml` to k8s cluster.

`./demo.yaml` includes a Deployment to hold the monitor scritps, a Service and ServiceMonitor to export the metrics.