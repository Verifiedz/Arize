#!/bin/bash 
set -e

GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

if ! docker info > /dev/null 2>&1; then
  echo "Error: Docker is not running. Restart docker and try again"
  exit 1 
fi

apikey=""
echo "Initializing dev environment"
cd ../docker
if [[ -f ./.env ]]; then # spacing
   echo ".env file exits"
else
  touch .env
  echo -e "${YELLOW} Enter APIKey:${NC}"
  read apikey
  echo  "ANTHROPIC_API_KEY=${apikey}" > .env # > creates/overwrite file >> appends to it $(system to execute cmd) 
  chmod 600 .env
fi




#Starting Docker enviroment via docker-compose.yml

echo -e "${GREEN}Starting up docker container session...${NC}"
docker compose run --build --rm claude # note using the -d flag runs it as a detatched process



