#!/bin/bash

set -e

echo "Deploying AgentMux Server to Bastion via SSM..."

# Get bastion instance ID
INSTANCE_ID=$(python -c "import boto3; ec2 = boto3.client('ec2', region_name='us-east-1'); r = ec2.describe_instances(Filters=[{'Name': 'tag:Name', 'Values': ['*bastion*']}, {'Name': 'instance-state-name', 'Values': ['running']}]); print(r['Reservations'][0]['Instances'][0]['InstanceId'])")

echo "Instance ID: $INSTANCE_ID"

# Colors for output
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

echo -e "${BLUE}[1/5] Creating deployment directory on bastion...${NC}"
aws ssm send-command \
  --region us-east-1 \
  --instance-ids "$INSTANCE_ID" \
  --document-name "AWS-RunShellScript" \
  --parameters 'commands=["sudo mkdir -p /opt/agentmux-server && sudo chown ec2-user:ec2-user /opt/agentmux-server"]' \
  --output text

sleep 5

echo -e "${BLUE}[2/5] Packaging and uploading source files...${NC}"
cd packages/agentmux-server
tar czf /tmp/agentmux-server.tar.gz \
  --exclude='node_modules' \
  --exclude='.git' \
  --exclude='dist' \
  .
cd ../..

# Upload to S3 temporarily
BUCKET="a5af-artifacts"
aws s3 cp /tmp/agentmux-server.tar.gz s3://$BUCKET/deploy/agentmux-server.tar.gz --region us-east-1

echo -e "${BLUE}[3/5] Downloading and extracting on bastion...${NC}"
COMMAND_ID=$(aws ssm send-command \
  --region us-east-1 \
  --instance-ids "$INSTANCE_ID" \
  --document-name "AWS-RunShellScript" \
  --parameters 'commands=["aws s3 cp s3://a5af-artifacts/deploy/agentmux-server.tar.gz /tmp/","tar xzf /tmp/agentmux-server.tar.gz -C /opt/agentmux-server","rm /tmp/agentmux-server.tar.gz","ls -la /opt/agentmux-server/"]' \
  --query "Command.CommandId" \
  --output text)

aws ssm wait command-executed --region us-east-1 --command-id "$COMMAND_ID" --instance-id "$INSTANCE_ID"
aws ssm get-command-invocation --region us-east-1 --command-id "$COMMAND_ID" --instance-id "$INSTANCE_ID" --query "StandardOutputContent" --output text

sleep 5

echo -e "${BLUE}[4/5] Installing dependencies and building...${NC}"
COMMAND_ID=$(aws ssm send-command \
  --region us-east-1 \
  --instance-ids "$INSTANCE_ID" \
  --document-name "AWS-RunShellScript" \
  --parameters 'commands=["cd /opt/agentmux-server","npm install","npm run build","npm prune --production"]' \
  --query "Command.CommandId" \
  --output text)

echo "Waiting for build to complete (Command ID: $COMMAND_ID)..."
aws ssm wait command-executed \
  --region us-east-1 \
  --command-id "$COMMAND_ID" \
  --instance-id "$INSTANCE_ID"

# Get build output
aws ssm get-command-invocation \
  --region us-east-1 \
  --command-id "$COMMAND_ID" \
  --instance-id "$INSTANCE_ID" \
  --query "StandardOutputContent" \
  --output text

echo -e "${BLUE}[5/5] Creating and starting systemd service...${NC}"
# Create systemd unit file using cat with multiple echo commands
COMMAND_ID=$(aws ssm send-command \
  --region us-east-1 \
  --instance-ids "$INSTANCE_ID" \
  --document-name "AWS-RunShellScript" \
  --parameters commands='[
    "sudo bash -c \"cat > /etc/systemd/system/agentmux-server.service << EOF\n[Unit]\nDescription=AgentMux WebSocket Server\nAfter=network.target\n\n[Service]\nType=simple\nUser=ec2-user\nWorkingDirectory=/opt/agentmux-server\nEnvironment=NODE_ENV=production\nEnvironment=PORT=8443\nEnvironment=SECRET_NAME=services/infra\nEnvironment=MESSAGES_TABLE_NAME=agentmux-messages-prod\nEnvironment=AGENTS_TABLE_NAME=agentmux-agents-prod\nEnvironment=AWS_REGION=us-east-1\nExecStart=/usr/bin/node dist/index.js\nRestart=always\nRestartSec=10\nStandardOutput=journal\nStandardError=journal\nSyslogIdentifier=agentmux-server\n\n[Install]\nWantedBy=multi-user.target\nEOF\n\"",
    "sudo systemctl daemon-reload",
    "sudo systemctl enable agentmux-server",
    "sudo systemctl restart agentmux-server",
    "sleep 3",
    "sudo systemctl status agentmux-server --no-pager"
  ]' \
  --query "Command.CommandId" \
  --output text)

echo "Waiting for service configuration..."
aws ssm wait command-executed \
  --region us-east-1 \
  --command-id "$COMMAND_ID" \
  --instance-id "$INSTANCE_ID"

# Get service status
aws ssm get-command-invocation \
  --region us-east-1 \
  --command-id "$COMMAND_ID" \
  --instance-id "$INSTANCE_ID" \
  --query "StandardOutputContent" \
  --output text

# Cleanup S3
aws s3 rm s3://$BUCKET/deploy/agentmux-server.tar.gz --region us-east-1
rm /tmp/agentmux-server.tar.gz

echo -e "${GREEN}Deployment complete!${NC}"
echo ""
echo "View logs with:"
echo "aws ssm start-session --target $INSTANCE_ID --region us-east-1 --document-name AWS-StartInteractiveCommand --parameters command='sudo journalctl -u agentmux-server -f'"
