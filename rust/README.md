# QUICK START

Example rust jitter. 
It subscribes to orders over Ws and tries to fill with some margin from oracle prices.  
It is provided as an example show casing the usage of drift-rs + jit-proxy.  

## Run
1. Create .env file in rust/ with your RPC_URL (example uses Helius) and your PRIVATE_KEY (as a [u8, u8, u8, u8])
2. run `bash run.sh` to start the jitter
