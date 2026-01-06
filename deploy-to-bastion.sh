#!/bin/bash

set -e

echo "Deploying AgentMux Server to Bastion..."

# Configuration
BASTION_HOST="ec2-34-192-240-117.compute-1.amazonaws.com"
DEPLOY_DIR="/opt/agentmux-server"
SOURCE_DIR="packages/agentmux-server"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}[1/6] Creating deployment directory on bastion...${NC}"
ssh ec2-user@${BASTION_HOST} "sudo mkdir -p ${DEPLOY_DIR} && sudo chown ec2-user:ec2-user ${DEPLOY_DIR}"

echo -e "${BLUE}[2/6] Copying source files to bastion...${NC}"
rsync -av --delete \
  --exclude 'node_modules' \
  --exclude '.git' \
  ${SOURCE_DIR}/ ec2-user@${BASTION_HOST}:${DEPLOY_DIR}/

echo -e "${BLUE}[3/6] Installing dependencies and building on bastion...${NC}"
ssh ec2-user@${BASTION_HOST} << 'ENDSSH'
set -e
cd /opt/agentmux-server

# Install dependencies
npm install --production=false

# Build TypeScript
npm run build

# Clean up dev dependencies
npm prune --production

echo "Build complete"
ENDSSH

echo -e "${BLUE}[4/6] Creating systemd service...${NC}"
ssh ec2-user@${BASTION_HOST} << 'ENDSSH'
set -e

# Create systemd service file
sudo tee /etc/systemd/system/agentmux-server.service > /dev/null << 'EOF'
[Unit]
Description=AgentMux WebSocket Server
After=network.target

[Service]
Type=simple
User=ec2-user
WorkingDirectory=/opt/agentmux-server
Environment=NODE_ENV=production
Environment=PORT=8443
Environment=SECRET_NAME=services/infra
Environment=MESSAGES_TABLE_NAME=agentmux-messages-prod
Environment=AGENTS_TABLE_NAME=agentmux-agents-prod
Environment=AWS_REGION=us-east-1
ExecStart=/usr/bin/node dist/index.js
Restart=always
RestartSec=10
StandardOutput=journal
StandardError=journal
SyslogIdentifier=agentmux-server

[Install]
WantedBy=multi-user.target
EOF

echo "Systemd service created"
ENDSSH

echo -e "${BLUE}[5/6] Enabling and starting service...${NC}"
ssh ec2-user@${BASTION_HOST} << 'ENDSSH'
set -e

# Reload systemd
sudo systemctl daemon-reload

# Enable service to start on boot
sudo systemctl enable agentmux-server

# Stop if running
sudo systemctl stop agentmux-server || true

# Start service
sudo systemctl start agentmux-server

# Show status
sudo systemctl status agentmux-server --no-pager

echo "Service started"
ENDSSH

echo -e "${BLUE}[6/6] Verifying deployment...${NC}"
ssh ec2-user@${BASTION_HOST} << 'ENDSSH'
set -e

# Wait a moment for service to start
sleep 2

# Check if service is running
if sudo systemctl is-active --quiet agentmux-server; then
  echo "✓ Service is running"
else
  echo "✗ Service failed to start"
  sudo journalctl -u agentmux-server -n 50 --no-pager
  exit 1
fi

# Check if listening on port 8443
if sudo ss -tlnp | grep -q :8443; then
  echo "✓ Listening on port 8443"
else
  echo "✗ Not listening on port 8443"
  exit 1
fi

echo "Deployment verification complete"
ENDSSH

echo -e "${GREEN}Deployment complete!${NC}"
echo ""
echo "Service status: ssh ec2-user@${BASTION_HOST} sudo systemctl status agentmux-server"
echo "View logs: ssh ec2-user@${BASTION_HOST} sudo journalctl -u agentmux-server -f"
echo "Restart: ssh ec2-user@${BASTION_HOST} sudo systemctl restart agentmux-server"
