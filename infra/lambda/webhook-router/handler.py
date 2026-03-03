"""
WaveMux Webhook Router Lambda Handler

Routes incoming webhooks to WaveMux terminal instances via WebSocket.
"""

import json
import os
import time
import hmac
import hashlib
import re
from typing import Dict, Any, List, Optional
from decimal import Decimal

import boto3
from boto3.dynamodb.conditions import Key
from botocore.exceptions import ClientError

# Initialize AWS clients
dynamodb = boto3.resource('dynamodb')
apigateway_client = boto3.client('apigatewaymanagementapi')
secrets_client = boto3.client('secretsmanager')

# Environment variables
WEBHOOK_CONFIG_TABLE = os.environ['WEBHOOK_CONFIG_TABLE']
CONNECTION_TABLE = os.environ['CONNECTION_TABLE']
ENVIRONMENT = os.environ['ENVIRONMENT']
SECRET_NAME = os.environ.get('SECRET_NAME', 'services/prod')
PROJECT_NAME = os.environ.get('PROJECT_NAME', 'wavemux')

# DynamoDB tables
webhook_config_table = dynamodb.Table(WEBHOOK_CONFIG_TABLE)
connection_table = dynamodb.Table(CONNECTION_TABLE)

# Cache for secrets
_secrets_cache = {}
_cache_timestamp = 0
CACHE_TTL = 300  # 5 minutes


def lambda_handler(event: Dict[str, Any], context: Any) -> Dict[str, Any]:
    """
    Main webhook router handler.

    Routes:
    - POST /webhook/{provider} - Receive webhook from external service
    - POST /register - Register terminal subscription
    - POST /unregister - Remove terminal subscription
    - GET /health - Health check
    - WebSocket $connect - Establish connection from WaveMux instance
    - WebSocket $disconnect - Clean up connection
    """

    print(f"Event: {json.dumps(event)}")

    # WebSocket routes
    route_key = event.get('requestContext', {}).get('routeKey')
    if route_key == '$connect':
        return handle_websocket_connect(event)
    elif route_key == '$disconnect':
        return handle_websocket_disconnect(event)

    # HTTP routes
    if event.get('requestContext', {}).get('http'):
        http_method = event['requestContext']['http']['method']
        path = event['requestContext']['http']['path']

        if http_method == 'GET' and path == '/health':
            return handle_health_check(event)
        elif http_method == 'POST':
            if path.startswith('/webhook/'):
                return handle_webhook_delivery(event)
            elif path == '/register':
                return handle_registration(event)
            elif path == '/unregister':
                return handle_unregistration(event)

    return {
        'statusCode': 404,
        'body': json.dumps({'error': 'Not Found'})
    }


def handle_health_check(event: Dict[str, Any]) -> Dict[str, Any]:
    """Health check endpoint."""
    return {
        'statusCode': 200,
        'body': json.dumps({
            'status': 'healthy',
            'service': 'wavemux-webhook-router',
            'environment': ENVIRONMENT,
            'timestamp': int(time.time())
        })
    }


def handle_webhook_delivery(event: Dict[str, Any]) -> Dict[str, Any]:
    """Process incoming webhook and route to subscribed terminals."""

    try:
        # Extract provider from path
        path = event['requestContext']['http']['path']
        provider = path.split('/')[-1]

        # Parse webhook payload
        body_str = event.get('body', '{}')
        if event.get('isBase64Encoded', False):
            import base64
            body_str = base64.b64decode(body_str).decode('utf-8')

        body = json.loads(body_str)
        headers = event.get('headers', {})

        # Validate webhook signature
        if not validate_webhook_signature(provider, headers, body_str):
            print(f"Invalid signature for provider: {provider}")
            return {
                'statusCode': 401,
                'body': json.dumps({'error': 'Invalid signature'})
            }

        # Determine event type
        event_type = extract_event_type(provider, headers, body)
        print(f"Webhook received: provider={provider}, event_type={event_type}")

        # Query DynamoDB for subscribed terminals
        subscriptions = query_subscriptions(provider, event_type)

        if not subscriptions:
            print(f"No subscriptions found for {provider}/{event_type}")
            return {
                'statusCode': 200,
                'body': json.dumps({
                    'message': 'Webhook received',
                    'delivered': 0,
                    'subscriptions': 0
                })
            }

        # Route to each subscribed terminal
        delivered = 0
        errors = []

        for subscription in subscriptions:
            try:
                # Check filters
                if not matches_filters(subscription.get('filters', {}), body):
                    continue

                # Render command template
                command = render_command_template(
                    subscription['commandTemplate'],
                    body
                )

                # Send to WaveMux instance via WebSocket
                success = send_to_terminal(
                    subscription['workspaceId'],
                    subscription['terminalId'],
                    command
                )

                if success:
                    delivered += 1
                else:
                    errors.append(f"Failed to deliver to {subscription['terminalId']}")

            except Exception as e:
                print(f"Error processing subscription {subscription.get('subscriptionId')}: {e}")
                errors.append(str(e))

        return {
            'statusCode': 200,
            'body': json.dumps({
                'message': 'Webhook processed',
                'delivered': delivered,
                'total_subscriptions': len(subscriptions),
                'errors': errors if errors else None
            })
        }

    except Exception as e:
        print(f"Error handling webhook: {e}")
        return {
            'statusCode': 500,
            'body': json.dumps({'error': str(e)})
        }


def validate_webhook_signature(provider: str, headers: Dict, body: str) -> bool:
    """Validate webhook signature based on provider."""

    if provider == 'github':
        signature = headers.get('x-hub-signature-256', '')
        if not signature:
            return False

        secret = get_webhook_secret(provider)
        if not secret:
            print(f"No secret found for provider: {provider}")
            return True  # Allow in dev/testing

        expected = 'sha256=' + hmac.new(
            secret.encode(),
            body.encode(),
            hashlib.sha256
        ).hexdigest()

        return hmac.compare_digest(signature, expected)

    # Add more providers as needed
    # For custom webhooks, could use a shared secret in Authorization header
    return True


def extract_event_type(provider: str, headers: Dict, body: Dict) -> str:
    """Extract event type from webhook payload."""

    if provider == 'github':
        return headers.get('x-github-event', 'unknown')

    # Default: look for 'event' or 'type' in body
    return body.get('event', body.get('type', 'unknown'))


def query_subscriptions(provider: str, event_type: str) -> List[Dict]:
    """Query DynamoDB for matching subscriptions."""

    try:
        response = webhook_config_table.query(
            IndexName='ProviderEventIndex',
            KeyConditionExpression=Key('provider').eq(provider) & Key('eventType').eq(event_type),
            FilterExpression='enabled = :enabled',
            ExpressionAttributeValues={
                ':enabled': True
            }
        )

        items = response.get('Items', [])

        # Convert Decimal to int/float for JSON serialization
        return [convert_decimals(item) for item in items]

    except Exception as e:
        print(f"Error querying subscriptions: {e}")
        return []


def matches_filters(filters: Dict, webhook_data: Dict) -> bool:
    """Check if webhook data matches subscription filters."""

    if not filters:
        return True

    for key, expected_value in filters.items():
        # Navigate nested keys using dot notation
        actual_value = get_nested_value(webhook_data, key)

        if isinstance(expected_value, list):
            # If filter is a list, check if actual value is in list
            if actual_value not in expected_value:
                return False
        elif isinstance(expected_value, str) and '*' in expected_value:
            # Simple wildcard matching
            pattern = expected_value.replace('*', '.*')
            if not re.match(pattern, str(actual_value)):
                return False
        else:
            # Exact match
            if actual_value != expected_value:
                return False

    return True


def send_to_terminal(workspace_id: str, terminal_id: str, command: str) -> bool:
    """Send command to terminal via WebSocket connection."""

    # Look up active WebSocket connection for workspace
    connection = get_active_connection(workspace_id)

    if not connection:
        print(f"No active connection for workspace: {workspace_id}")
        return False

    connection_id = connection['connectionId']

    # Get API Gateway endpoint from connection context
    domain_name = connection.get('domainName')
    stage = connection.get('stage', ENVIRONMENT)

    if not domain_name:
        print(f"No domain name in connection record")
        return False

    # Initialize API Gateway Management API client
    endpoint_url = f"https://{domain_name}/{stage}"
    apigw_client = boto3.client(
        'apigatewaymanagementapi',
        endpoint_url=endpoint_url
    )

    # Send message via API Gateway WebSocket
    try:
        message = {
            'action': 'inject',
            'terminalId': terminal_id,
            'command': command,
            'timestamp': int(time.time())
        }

        apigw_client.post_to_connection(
            ConnectionId=connection_id,
            Data=json.dumps(message).encode('utf-8')
        )

        print(f"Sent command to terminal {terminal_id} via connection {connection_id}")
        return True

    except apigw_client.exceptions.GoneException:
        print(f"Connection {connection_id} is gone, cleaning up")
        # Clean up stale connection
        connection_table.delete_item(Key={'connectionId': connection_id})
        return False

    except Exception as e:
        print(f"Error sending to connection {connection_id}: {e}")
        return False


def get_active_connection(workspace_id: str) -> Optional[Dict]:
    """Get active WebSocket connection for workspace."""

    try:
        response = connection_table.query(
            IndexName='WorkspaceIndex',
            KeyConditionExpression=Key('workspaceId').eq(workspace_id),
            Limit=1,
            ScanIndexForward=False  # Get most recent
        )

        items = response.get('Items', [])
        if items:
            return convert_decimals(items[0])

        return None

    except Exception as e:
        print(f"Error getting connection for workspace {workspace_id}: {e}")
        return None


def render_command_template(template: str, webhook_data: Dict) -> str:
    """Render command template with webhook data."""

    def replace_token(match):
        path = match.group(1)
        value = get_nested_value(webhook_data, path)
        return str(value) if value is not None else ''

    # Simple template engine supporting {{path.to.value}} syntax
    result = re.sub(r'\{\{([^}]+)\}\}', replace_token, template)

    # Ensure command ends with newline if not present
    if not result.endswith('\n'):
        result += '\n'

    return result


def get_nested_value(data: Dict, path: str) -> Any:
    """Get nested value from dict using dot notation."""

    keys = path.split('.')
    value = data

    for key in keys:
        if isinstance(value, dict):
            value = value.get(key)
        else:
            return None

    return value


def handle_registration(event: Dict[str, Any]) -> Dict[str, Any]:
    """Register a new terminal subscription."""

    try:
        body_str = event.get('body', '{}')
        if event.get('isBase64Encoded', False):
            import base64
            body_str = base64.b64decode(body_str).decode('utf-8')

        body = json.loads(body_str)

        # Validate required fields
        required = ['workspaceId', 'terminalId', 'subscription']
        if not all(k in body for k in required):
            return {
                'statusCode': 400,
                'body': json.dumps({
                    'error': 'Missing required fields',
                    'required': required
                })
            }

        subscription = body['subscription']

        # Store in DynamoDB
        item = {
            'subscriptionId': subscription['id'],
            'workspaceId': body['workspaceId'],
            'terminalId': body['terminalId'],
            'provider': subscription['provider'],
            'eventType': subscription['events'][0] if subscription['events'] else 'unknown',
            'events': subscription['events'],
            'filters': subscription.get('filters', {}),
            'commandTemplate': subscription['commandTemplate'],
            'enabled': subscription.get('enabled', True),
            'createdAt': int(time.time()),
        }

        webhook_config_table.put_item(Item=item)

        print(f"Registered subscription: {subscription['id']}")

        return {
            'statusCode': 200,
            'body': json.dumps({
                'message': 'Subscription registered',
                'subscriptionId': subscription['id']
            })
        }

    except Exception as e:
        print(f"Error registering subscription: {e}")
        return {
            'statusCode': 500,
            'body': json.dumps({'error': str(e)})
        }


def handle_unregistration(event: Dict[str, Any]) -> Dict[str, Any]:
    """Remove a terminal subscription."""

    try:
        body_str = event.get('body', '{}')
        if event.get('isBase64Encoded', False):
            import base64
            body_str = base64.b64decode(body_str).decode('utf-8')

        body = json.loads(body_str)
        subscription_id = body.get('subscriptionId')

        if not subscription_id:
            return {
                'statusCode': 400,
                'body': json.dumps({'error': 'Missing subscriptionId'})
            }

        webhook_config_table.delete_item(Key={'subscriptionId': subscription_id})

        print(f"Unregistered subscription: {subscription_id}")

        return {
            'statusCode': 200,
            'body': json.dumps({'message': 'Subscription removed'})
        }

    except Exception as e:
        print(f"Error unregistering subscription: {e}")
        return {
            'statusCode': 500,
            'body': json.dumps({'error': str(e)})
        }


def handle_websocket_connect(event: Dict[str, Any]) -> Dict[str, Any]:
    """Handle WebSocket connection from WaveMux instance."""

    connection_id = event['requestContext']['connectionId']
    domain_name = event['requestContext']['domainName']
    stage = event['requestContext']['stage']

    # Extract workspaceId and auth token from query params
    query_params = event.get('queryStringParameters') or {}
    workspace_id = query_params.get('workspaceId')
    auth_token = query_params.get('token')

    if not workspace_id:
        print("Missing workspaceId in connection request")
        return {'statusCode': 400}

    if not auth_token:
        print("Missing auth token in connection request")
        return {'statusCode': 401}

    # Validate authentication token
    if not validate_auth_token(workspace_id, auth_token):
        print(f"Invalid auth token for workspace: {workspace_id}")
        return {'statusCode': 401}

    # Store connection in DynamoDB
    try:
        connection_table.put_item(
            Item={
                'connectionId': connection_id,
                'workspaceId': workspace_id,
                'domainName': domain_name,
                'stage': stage,
                'connectedAt': int(time.time()),
                'ttl': int(time.time()) + 86400,  # 24 hour TTL
            }
        )

        print(f"WebSocket connected: {connection_id} for workspace {workspace_id}")

        return {'statusCode': 200}

    except Exception as e:
        print(f"Error storing connection: {e}")
        return {'statusCode': 500}


def handle_websocket_disconnect(event: Dict[str, Any]) -> Dict[str, Any]:
    """Clean up WebSocket connection."""

    connection_id = event['requestContext']['connectionId']

    try:
        connection_table.delete_item(Key={'connectionId': connection_id})
        print(f"WebSocket disconnected: {connection_id}")
        return {'statusCode': 200}

    except Exception as e:
        print(f"Error deleting connection: {e}")
        return {'statusCode': 500}


def validate_auth_token(workspace_id: str, token: str) -> bool:
    """Validate workspace authentication token."""

    # Get expected token from Secrets Manager
    secret = get_webhook_secret('default')

    if not secret:
        # In dev/testing, allow any token
        return True

    # Simple token validation - in production, use JWT or similar
    expected_token = hmac.new(
        secret.encode(),
        workspace_id.encode(),
        hashlib.sha256
    ).hexdigest()

    return hmac.compare_digest(token, expected_token)


def get_webhook_secret(key: str) -> str:
    """
    Get webhook secret from cache or Secrets Manager.

    Secrets are stored in services/prod with structure:
    {
      "wavemux": {
        "GITHUB_WEBHOOK_SECRET": "...",
        "CUSTOM_WEBHOOK_SECRET": "...",
        "DEFAULT_AUTH_SECRET": "..."
      }
    }
    """

    global _secrets_cache, _cache_timestamp

    # Check cache
    current_time = time.time()
    if _secrets_cache and (current_time - _cache_timestamp) < CACHE_TTL:
        project_secrets = _secrets_cache.get(PROJECT_NAME, {})
        # Map key to secret name
        secret_key_map = {
            'github': 'GITHUB_WEBHOOK_SECRET',
            'custom': 'CUSTOM_WEBHOOK_SECRET',
            'default': 'DEFAULT_AUTH_SECRET',
        }
        secret_key = secret_key_map.get(key, key.upper() + '_SECRET')
        return project_secrets.get(secret_key, '')

    # Fetch from Secrets Manager
    try:
        response = secrets_client.get_secret_value(SecretId=SECRET_NAME)
        secret_string = response['SecretString']
        all_secrets = json.loads(secret_string)

        _secrets_cache = all_secrets
        _cache_timestamp = current_time

        # Get project-specific secrets
        project_secrets = all_secrets.get(PROJECT_NAME, {})

        # Map key to secret name
        secret_key_map = {
            'github': 'GITHUB_WEBHOOK_SECRET',
            'custom': 'CUSTOM_WEBHOOK_SECRET',
            'default': 'DEFAULT_AUTH_SECRET',
        }
        secret_key = secret_key_map.get(key, key.upper() + '_SECRET')
        return project_secrets.get(secret_key, '')

    except Exception as e:
        print(f"Error fetching secrets from {SECRET_NAME}: {e}")
        return ''


def convert_decimals(obj):
    """Convert DynamoDB Decimal types to int/float for JSON serialization."""

    if isinstance(obj, list):
        return [convert_decimals(i) for i in obj]
    elif isinstance(obj, dict):
        return {k: convert_decimals(v) for k, v in obj.items()}
    elif isinstance(obj, Decimal):
        if obj % 1 == 0:
            return int(obj)
        else:
            return float(obj)
    else:
        return obj
