"""Client for the actions broker: short-lived credentials for this workflow run.

The job proves which workflow it is with GitHub's OIDC token; the broker
decides what that workflow may receive and returns it. Every value it returns
is masked before anything else runs, so it can't appear in this public log.

Modes (env MODE):
  grant   POST /grant; writes the response to GRANT_FILE (mode 600)
  aws     POST /grant; exports the granted AWS credentials to later steps
  result  POST /result with the contents of RESULT_FILE

Env: BROKER_URL, ENTRY, CONTEXT (JSON, grant/aws), GRANT_FILE, RESULT_FILE.
Retries transient failures (throttling, 5xx, network) for about two minutes,
then fails the step. Never prints a credential or a response body.
"""
import json
import os
import secrets
import sys
import time
import urllib.error
import urllib.request

AUDIENCE = 'a5af-actions-broker'
DELAYS = [5, 10, 20, 30, 30, 30]
FINAL = (400, 401, 403, 404, 409, 413)


def out(name, value):
    with open(os.environ['GITHUB_OUTPUT'], 'a', encoding='utf-8') as f:
        f.write(f'{name}={value}\n')


def mask(value):
    for line in str(value).splitlines():
        if len(line.strip()) >= 6:
            print(f'::add-mask::{line.strip()}', flush=True)


def mask_all(obj):
    if isinstance(obj, dict):
        for v in obj.values():
            mask_all(v)
    elif isinstance(obj, list):
        for v in obj:
            mask_all(v)
    elif isinstance(obj, str):
        mask(obj)
        try:
            mask_all(json.loads(obj))
        except ValueError:
            pass


def oidc_token():
    url = os.environ['ACTIONS_ID_TOKEN_REQUEST_URL'] + '&audience=' + AUDIENCE
    req = urllib.request.Request(url, headers={'Authorization': 'bearer ' + os.environ['ACTIONS_ID_TOKEN_REQUEST_TOKEN']})
    with urllib.request.urlopen(req, timeout=15) as r:
        token = json.load(r)['value']
    mask(token)
    return token


def call(path, body):
    url = os.environ['BROKER_URL'].rstrip('/') + path
    data = json.dumps(body).encode()
    status = 0
    for delay in DELAYS + [None]:
        try:
            req = urllib.request.Request(url, data=data, method='POST', headers={
                'Authorization': 'Bearer ' + oidc_token(), 'Content-Type': 'application/json'})
            with urllib.request.urlopen(req, timeout=40) as r:
                return r.status, json.load(r)
        except urllib.error.HTTPError as e:
            status = e.code
            try:
                payload = json.load(e)
            except ValueError:
                payload = {}
            if status in FINAL:
                return status, payload
        except (urllib.error.URLError, TimeoutError, OSError):
            status = 0
        if delay is None:
            break
        print(f'broker answered {status or "nothing"}; retrying in {delay}s', flush=True)
        time.sleep(delay)
    return status, {'error': 'broker unavailable'}


def fail(msg):
    print(f'::error::{msg}', flush=True)
    sys.exit(1)


def grant():
    body = {'entry': os.environ['ENTRY'], 'request_id': secrets.token_hex(12),
            'context': json.loads(os.environ.get('CONTEXT') or '{}')}
    status, data = call('/grant', body)
    mask_all({k: v for k, v in data.items() if k in ('secrets', 'github', 'aws', 'settings')})
    if status != 200:
        fail(f'broker refused the grant ({status}: {data.get("error", "")})')
    if data.get('skip'):
        print(f'broker: skip ({data["skip"]})', flush=True)
        out('skip', 'true')
        return data
    out('skip', '')
    return data


def main():
    mode = os.environ['MODE']
    if mode in ('grant', 'aws'):
        data = grant()
        if mode == 'grant':
            path = os.environ['GRANT_FILE']
            fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
            with os.fdopen(fd, 'w', encoding='utf-8') as f:
                json.dump(data, f)
            out('grant-file', path)
        elif not data.get('skip'):
            aws = data.get('aws') or fail('the grant has no AWS credentials')
            with open(os.environ['GITHUB_ENV'], 'a', encoding='utf-8') as f:
                f.write(f"AWS_ACCESS_KEY_ID={aws['access_key_id']}\n")
                f.write(f"AWS_SECRET_ACCESS_KEY={aws['secret_access_key']}\n")
                f.write(f"AWS_SESSION_TOKEN={aws['session_token']}\n")
                f.write('AWS_REGION=us-east-1\n')
        return
    if mode == 'result':
        with open(os.environ['RESULT_FILE'], encoding='utf-8') as f:
            result = f.read()
        status, data = call('/result', {'entry': os.environ['ENTRY'], 'result': result})
        if status == 409 and 'already written' in str(data.get('error', '')):
            print('broker: result was already accepted (an earlier attempt got through)', flush=True)
            return
        if status != 200:
            fail(f'broker refused the result ({status}: {data.get("error", "")})')
        print(f"broker: result {data.get('status')}, {data.get('replayed', 0)} write(s) posted, "
              f"{data.get('skipped', 0)} skipped", flush=True)
        return
    fail(f'unknown mode {mode}')


if __name__ == '__main__':
    main()
