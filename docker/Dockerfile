
## NOTE TO RUN THIS IN THE VM ENVIRONMENT

#Dockerfile Defining the isolated environment

FROM debian:bookworm-slim
#working directory that we are going to use claude in
WORKDIR /workspace

#Debian based machine needed packages install

RUN apt-get update && apt install -y git nodejs npm rustc cargo curl build-essential 

#Non Root User
RUN useradd -m -u 1001 claude


RUN chown claude:claude /workspace
#Installing Claude code

RUN npm install -g @anthropic-ai/claude-code

## NOTE docker runs all cmds as root user unless we explicitly switch
USER claude
#define the entrypoint

ENTRYPOINT ["claude"]

