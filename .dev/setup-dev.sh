#!/bin/bash 
set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

if ! docker info > /dev/null 2>&1; then
  echo "Error: Docker is not running. Restart docker and try again"
  exit 1 
fi

echo "Initializing dev environment"
cd "$(dirname "$0")/.."
# Ensure the Claude config file exists so Docker mounts it as a file, not a directory
if [ ! -d "docker/claude-config" ]; then
	mkdir docker/claude-config
fi
# Starting Docker environment via docker-compose.yml
echo -e "${GREEN}Starting up docker container session...${NC}"
docker compose -f docker/docker-compose.yml run --build --rm claude 
# note using the -d flag runs it as a detatched process
