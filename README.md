# Azure bastion tunnel

command line utility for creating tunnels to azure bastion to vm


## build 

`cargo build`

## run

`cargo run -- --resource-group <resource-group-name> --vm fc --bastion <bastion-name> --remote-port 22 --local-port 8080`

### DISCLAIMER

For education purpose only

Azure currently is providing any official documentation for bastion. Current project is reverse engineered from [cli](https://github.com/Azure/azure-cli/blob/dev/src/azure-cli/azure/cli/command_modules/network/tunnel.py#L139)

