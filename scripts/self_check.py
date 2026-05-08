#!/usr/bin/env python3
import json, subprocess, sys
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(ROOT))
from cxs import core

def main():
    print(json.dumps(core.cxs_self_check(str(ROOT)), ensure_ascii=False, indent=2))
    proc = subprocess.Popen([sys.executable, str(ROOT/'mcp'/'cxs_mcp_server.py')], stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    assert proc.stdin and proc.stdout
    proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':1,'method':'initialize','params':{'protocolVersion':'2025-06-18','capabilities':{},'clientInfo':{'name':'self','version':'0'}}})+'\n')
    proc.stdin.write(json.dumps({'jsonrpc':'2.0','method':'notifications/initialized'})+'\n')
    proc.stdin.write(json.dumps({'jsonrpc':'2.0','id':2,'method':'tools/list','params':{}})+'\n')
    proc.stdin.flush()
    a = proc.stdout.readline().strip(); b = proc.stdout.readline().strip()
    proc.stdin.close(); proc.terminate()
    try: proc.wait(timeout=2)
    except Exception: proc.kill(); proc.wait(timeout=2)
    if proc.stdout: proc.stdout.close()
    if proc.stderr: proc.stderr.close()
    print(a); print(b)
    assert '"cxs"' in b
    print('SELF_CHECK_OK')
    return 0
if __name__ == '__main__':
    raise SystemExit(main())
