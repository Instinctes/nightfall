#!/usr/bin/env python3
"""Real disposable Devnet HTTP checks. No wallet, internet or mainnet access."""
import json
import os
from pathlib import Path
import socket
import subprocess
import tempfile
import time

ROOT = Path(__file__).resolve().parents[1]
BIN = Path(os.environ.get('NIGHTFALL_LAB_BIN_DIR', ROOT / 'target/debug')) / 'nightfalld'

def port():
    with socket.socket() as s:
        s.bind(('127.0.0.1', 0))
        return s.getsockname()[1]

def read_all(s):
    data = b''
    while True:
        more = s.recv(65536)
        if not more:
            return data
        data += more

def request(light, payload):
    with socket.create_connection(('127.0.0.1', light), timeout=5) as s:
        s.sendall(payload)
        return read_all(s)

def post(body, extra=b''):
    return b'POST / HTTP/1.1\r\nHost: localhost\r\nContent-Length: ' + str(len(body)).encode() + b'\r\n' + extra + b'\r\n' + body

with tempfile.TemporaryDirectory(prefix='nightfall-light-boundary-') as tmp:
    p2p, rpc, light = port(), port(), port()
    env = {**os.environ, 'NIGHTFALL_PEERS_URL': ''}
    env.pop('NIGHTFALL_TRUST_LOOPBACK_PROXY', None)
    with open(Path(tmp) / 'node.log', 'wb') as log:
        node = subprocess.Popen([str(BIN), '--network', 'devnet', '--datadir', str(Path(tmp)/'node'),
            'run', '--listen', f'127.0.0.1:{p2p}', '--rpc-listen', f'127.0.0.1:{rpc}',
            '--mobile-listen', f'127.0.0.1:{light}', '--proxy', 'off'], env=env, stdout=log, stderr=log)
        sockets = []
        try:
            deadline = time.monotonic() + 30
            while True:
                try:
                    reply = request(light, b'GET /status HTTP/1.1\r\nHost: localhost\r\n\r\n')
                    if b'200 OK' in reply and json.loads(reply.split(b'\r\n\r\n', 1)[1])['result']['network'] == 'devnet':
                        break
                except (OSError, ValueError, KeyError):
                    pass
                assert time.monotonic() < deadline and node.poll() is None, 'node did not start'
                time.sleep(.1)
            body = b'{"method":"status","params":{},"id":1}'
            with socket.create_connection(('127.0.0.1', light), timeout=5) as s:
                s.sendall(b'POST / HTTP/1.1\r\nContent-Len')
                time.sleep(.03)
                s.sendall(b'gth: '+str(len(body)).encode()+b'\r\n\r\n'+body)
                assert b'200 OK' in read_all(s), 'split headers failed'
            for extra in (b'Content-Length: 1\r\n', b'Transfer-Encoding: chunked\r\n'):
                assert b'400 Bad Request' in request(light, post(body, extra))
            assert b'403 Forbidden' in request(light, post(b'{"method":"mine_one","params":{},"id":1}'))
            for _ in range(40):
                s = socket.create_connection(('127.0.0.1', light), timeout=5)
                s.settimeout(2)
                sockets.append(s)
                time.sleep(.005)
            # At most eight workers plus sixteen queued sockets remain admitted.
            refused = 0
            for s in sockets[24:]:
                try:
                    if s.recv(1) == b'':
                        refused += 1
                except ConnectionResetError:
                    refused += 1
            assert refused >= 16, f'expected bounded admission; only {refused} refused'
            for s in sockets:
                s.close()
            sockets.clear()
            deadline = time.monotonic() + 5
            while True:
                try:
                    assert b'200 OK' in request(light, b'GET /status HTTP/1.1\r\nHost: localhost\r\n\r\n')
                    break
                except OSError:
                    assert time.monotonic() < deadline
                    time.sleep(.1)
            print('PASS: real Devnet split headers, ambiguous framing, privileged-method refusal, bounded admission and recovery')
        finally:
            for s in sockets:
                s.close()
            node.terminate()
            try:
                node.wait(timeout=10)
            except subprocess.TimeoutExpired:
                node.kill()
                node.wait(timeout=10)
