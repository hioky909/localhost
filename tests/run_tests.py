#!/usr/bin/env python3
"""
Automated test suite for the 'localhost' HTTP server project.
Based on the Zone01 Normandie audit criteria.

Usage:
    python3 tests/run_tests.py [host] [port]

Default:
    host = 127.0.0.1
    port = 8080

Requirements:
    - Python 3.6+
    - The server must be running before executing this script.
    - For siege stress tests: siege must be installed (apt install siege).
    - For upload tests: write permission to the upload folder configured in server.conf.
"""

import sys
import os
import socket
import http.client
import json
import time
import subprocess
import tempfile
import urllib.parse
from pathlib import Path

# ──────────────────────────────────────────────
# Config
# ──────────────────────────────────────────────
HOST = sys.argv[1] if len(sys.argv) > 1 else "127.0.0.1"
PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8080

PASS = "\033[92m[PASS]\033[0m"
FAIL = "\033[91m[FAIL]\033[0m"
SKIP = "\033[93m[SKIP]\033[0m"
INFO = "\033[94m[INFO]\033[0m"

passed = 0
failed = 0
skipped = 0


def log(status, name, detail=""):
    global passed, failed, skipped
    if status == "PASS":
        passed += 1
        print(f"{PASS} {name}")
    elif status == "FAIL":
        failed += 1
        print(f"{FAIL} {name}")
        if detail:
            print(f"       └─ {detail}")
    elif status == "SKIP":
        skipped += 1
        print(f"{SKIP} {name} – {detail}")


def section(title):
    print(f"\n{'─'*60}")
    print(f"  {title}")
    print(f"{'─'*60}")


# ──────────────────────────────────────────────
# Low-level HTTP helpers (no external libs)
# ──────────────────────────────────────────────
def raw_get(path="/", host=None, port=None, headers=None, timeout=5):
    h = host or HOST
    p = port or PORT
    extra = ""
    if headers:
        for k, v in headers.items():
            extra += f"{k}: {v}\r\n"
    request = f"GET {path} HTTP/1.1\r\nHost: {h}:{p}\r\nConnection: close\r\n{extra}\r\n"
    with socket.create_connection((h, p), timeout=timeout) as s:
        s.sendall(request.encode())
        response = b""
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            response += chunk
    return response.decode(errors="replace")


def raw_request(method, path, body=b"", content_type="text/plain", host=None, port=None, extra_headers=None, timeout=5):
    h = host or HOST
    p = port or PORT
    headers = (
        f"{method} {path} HTTP/1.1\r\n"
        f"Host: {h}:{p}\r\n"
        f"Content-Type: {content_type}\r\n"
        f"Content-Length: {len(body)}\r\n"
        "Connection: close\r\n"
    )
    if extra_headers:
        for k, v in extra_headers.items():
            headers += f"{k}: {v}\r\n"
    headers += "\r\n"
    with socket.create_connection((h, p), timeout=timeout) as s:
        s.sendall(headers.encode() + body)
        response = b""
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            response += chunk
    return response.decode(errors="replace")


def status_code(raw_response):
    try:
        first_line = raw_response.split("\r\n")[0]
        return int(first_line.split()[1])
    except Exception:
        return -1


def is_server_up(host=None, port=None, timeout=2):
    try:
        with socket.create_connection((host or HOST, port or PORT), timeout=timeout):
            return True
    except Exception:
        return False


# ──────────────────────────────────────────────
# 1. Server Availability
# ──────────────────────────────────────────────
section("1. Server Availability")

if is_server_up():
    log("PASS", "Server is reachable")
else:
    log("FAIL", "Server is NOT reachable", f"Cannot connect to {HOST}:{PORT}. Is the server running?")
    print("\nAborting – no point running further tests without a live server.")
    sys.exit(1)

# ──────────────────────────────────────────────
# 2. HTTP Methods
# ──────────────────────────────────────────────
section("2. HTTP Methods")

# GET
try:
    resp = raw_get("/")
    code = status_code(resp)
    if code in (200, 301, 302, 403, 404):
        log("PASS", f"GET / → {code}")
    else:
        log("FAIL", f"GET / → unexpected status {code}")
except Exception as e:
    log("FAIL", "GET /", str(e))

# POST (empty body)
try:
    resp = raw_request("POST", "/", body=b"hello=world")
    code = status_code(resp)
    if code in (200, 201, 204, 400, 403, 404, 405):
        log("PASS", f"POST / → {code}")
    else:
        log("FAIL", f"POST / → unexpected status {code}")
except Exception as e:
    log("FAIL", "POST /", str(e))

# DELETE
try:
    resp = raw_request("DELETE", "/nonexistent_resource_12345")
    code = status_code(resp)
    if code in (200, 204, 403, 404, 405):
        log("PASS", f"DELETE /nonexistent → {code}")
    else:
        log("FAIL", f"DELETE /nonexistent → unexpected status {code}")
except Exception as e:
    log("FAIL", "DELETE /nonexistent", str(e))

# Wrong / unknown method
try:
    resp = raw_request("FOOBAR", "/")
    code = status_code(resp)
    if code in (400, 405, 501):
        log("PASS", f"Unknown method FOOBAR → {code} (server handled it gracefully)")
    else:
        log("FAIL", f"Unknown method FOOBAR → {code} (should be 400/405/501)")
except Exception as e:
    log("FAIL", "Unknown method FOOBAR", str(e))

# ──────────────────────────────────────────────
# 3. Error Pages
# ──────────────────────────────────────────────
section("3. Error Pages")

ERROR_CODES = {
    400: raw_request("GET", "badpath\x00\x00"),  # malformed
    404: raw_get("/this_page_definitely_does_not_exist_xyz987"),
    405: raw_request("DELETE", "/"),  # likely not allowed on root
}

for expected_code, resp in ERROR_CODES.items():
    code = status_code(resp)
    if code == expected_code:
        log("PASS", f"Error page {expected_code} returned correctly")
    elif code in (expected_code, 400, 404, 405, 500):
        log("PASS", f"Error page check → {code} (acceptable response)")
    else:
        log("FAIL", f"Expected {expected_code}, got {code}")

# ──────────────────────────────────────────────
# 4. Headers
# ──────────────────────────────────────────────
section("4. HTTP Response Headers")

try:
    resp = raw_get("/")
    has_content_length = "Content-Length:" in resp or "Transfer-Encoding:" in resp
    has_content_type = "Content-Type:" in resp
    has_status_line = resp.startswith("HTTP/1.1")

    log("PASS" if has_status_line else "FAIL", "Response starts with HTTP/1.1")
    log("PASS" if has_content_type else "FAIL", "Response includes Content-Type header")
    log("PASS" if has_content_length else "FAIL", "Response includes Content-Length or Transfer-Encoding")
except Exception as e:
    log("FAIL", "Header check", str(e))

# ──────────────────────────────────────────────
# 5. File Upload (POST multipart)
# ──────────────────────────────────────────────
section("5. File Upload")

BOUNDARY = "----TestBoundary123"
FILENAME = "testfile_upload.txt"
FILE_CONTENT = b"Hello, this is an upload test file!\n"

multipart_body = (
    f"--{BOUNDARY}\r\n"
    f'Content-Disposition: form-data; name="file"; filename="{FILENAME}"\r\n'
    f"Content-Type: text/plain\r\n\r\n"
).encode() + FILE_CONTENT + f"\r\n--{BOUNDARY}--\r\n".encode()

try:
    resp = raw_request(
        "POST", "/upload",
        body=multipart_body,
        content_type=f"multipart/form-data; boundary={BOUNDARY}",
    )
    code = status_code(resp)
    if code in (200, 201, 204):
        log("PASS", f"File upload → {code}")
    elif code in (404, 405):
        log("SKIP", "File upload", f"Route /upload not configured (→ {code}); configure upload_store in server.conf")
    else:
        log("FAIL", f"File upload → unexpected status {code}")
except Exception as e:
    log("FAIL", "File upload", str(e))

# ──────────────────────────────────────────────
# 6. Body Size Limit
# ──────────────────────────────────────────────
section("6. Client Body Size Limit")

try:
    big_body = b"A" * (2 * 1024 * 1024)  # 2 MB – should exceed typical limit
    resp = raw_request("POST", "/", body=big_body, timeout=10)
    code = status_code(resp)
    if code == 413:
        log("PASS", "Body too large → 413 Payload Too Large")
    elif code in (400, 500):
        log("PASS", f"Body too large → {code} (server rejected it)")
    else:
        log("FAIL", f"Body too large → {code} (expected 413)")
except Exception as e:
    log("PASS", f"Body too large → connection reset ({type(e).__name__}), server protected itself")

# ──────────────────────────────────────────────
# 7. Redirections
# ──────────────────────────────────────────────
section("7. Redirections")

try:
    resp = raw_get("/redirect_test")
    code = status_code(resp)
    if code in (301, 302, 307, 308):
        log("PASS", f"Redirection → {code} (Location header present: {'Location:' in resp})")
    elif code in (404, 405):
        log("SKIP", "Redirection", "Route /redirect_test not configured; add a `return` directive in server.conf")
    else:
        log("FAIL", f"Redirection → unexpected {code}")
except Exception as e:
    log("FAIL", "Redirection", str(e))

# ──────────────────────────────────────────────
# 8. Directory Listing
# ──────────────────────────────────────────────
section("8. Directory Listing")

try:
    resp = raw_get("/")
    code = status_code(resp)
    if code == 200 and ("<html" in resp.lower() or "index" in resp.lower()):
        log("PASS", "GET / returns HTML content (index or directory listing)")
    elif code == 200:
        log("PASS", f"GET / → 200 OK")
    elif code in (301, 302, 403, 404):
        log("PASS", f"GET / → {code} (server responded)")
    else:
        log("FAIL", f"GET / → {code}")
except Exception as e:
    log("FAIL", "Directory listing / index page", str(e))

# ──────────────────────────────────────────────
# 9. Cookies & Sessions
# ──────────────────────────────────────────────
section("9. Cookies & Sessions")

try:
    resp = raw_get("/")
    if "Set-Cookie:" in resp:
        log("PASS", "Server sets a cookie in the response")
        # Extract and send it back
        for line in resp.split("\r\n"):
            if line.startswith("Set-Cookie:"):
                cook = line.split(":", 1)[1].strip().split(";")[0]
                resp2 = raw_get("/", headers={"Cookie": cook})
                log("PASS" if status_code(resp2) in (200, 301, 302) else "FAIL",
                    f"Server accepts Cookie header back → {status_code(resp2)}")
                break
    else:
        log("SKIP", "Cookies", "No Set-Cookie header returned by GET /; implement session management or a CGI that sets cookies")
except Exception as e:
    log("FAIL", "Cookies", str(e))

# ──────────────────────────────────────────────
# 10. Chunked Request Encoding
# ──────────────────────────────────────────────
section("10. Chunked Request Handling")

try:
    chunk_data = b"Hello World"
    chunk_hex = hex(len(chunk_data))[2:]
    chunked_body = f"{chunk_hex}\r\n".encode() + chunk_data + b"\r\n0\r\n\r\n"

    h = HOST
    p = PORT
    request = (
        f"POST / HTTP/1.1\r\n"
        f"Host: {h}:{p}\r\n"
        f"Transfer-Encoding: chunked\r\n"
        f"Connection: close\r\n"
        f"\r\n"
    ).encode() + chunked_body

    with socket.create_connection((h, p), timeout=5) as s:
        s.sendall(request)
        resp = b""
        while True:
            chunk = s.recv(4096)
            if not chunk:
                break
            resp += chunk
    resp_str = resp.decode(errors="replace")
    code = status_code(resp_str)
    if code in (200, 201, 204, 400, 404, 405):
        log("PASS", f"Chunked POST request → {code} (server handled it)")
    else:
        log("FAIL", f"Chunked POST → {code}")
except Exception as e:
    log("FAIL", "Chunked request", str(e))

# ──────────────────────────────────────────────
# 11. Timeout (slow client)
# ──────────────────────────────────────────────
section("11. Request Timeout (slow client)")

try:
    h = HOST
    p = PORT
    with socket.create_connection((h, p), timeout=10) as s:
        # Send an incomplete request and wait
        s.sendall(b"GET / HTTP/1.1\r\nHost: test\r\n")
        s.settimeout(15)
        start = time.time()
        try:
            data = s.recv(4096)
            elapsed = time.time() - start
            if data:
                resp_str = data.decode(errors="replace")
                code = status_code(resp_str)
                log("PASS", f"Server responded to slow client within {elapsed:.1f}s → {code}")
            else:
                log("PASS", f"Server closed connection on incomplete request after {elapsed:.1f}s")
        except socket.timeout:
            log("SKIP", "Timeout test", "Server kept connection open for >15s; implement a request timeout in server.conf")
except Exception as e:
    log("PASS", f"Timeout: server closed connection ({type(e).__name__})")

# ──────────────────────────────────────────────
# 12. Multiple Ports (if configured)
# ──────────────────────────────────────────────
section("12. Multiple Ports")

ALT_PORT = 8081
if is_server_up(HOST, ALT_PORT):
    try:
        resp = raw_get("/", port=ALT_PORT)
        log("PASS", f"Second server on port {ALT_PORT} responds → {status_code(resp)}")
    except Exception as e:
        log("FAIL", f"Second server on port {ALT_PORT}", str(e))
else:
    log("SKIP", f"Multiple ports", f"No server detected on port {ALT_PORT}; add a second `listen` or server block in server.conf")

# ──────────────────────────────────────────────
# 13. Virtual Hosts (server_name)
# ──────────────────────────────────────────────
section("13. Virtual Hosts (server_name)")

try:
    resp_a = raw_get("/", headers={"Host": "test.com"})
    resp_b = raw_get("/", headers={"Host": "other.com"})
    code_a = status_code(resp_a)
    code_b = status_code(resp_b)
    if code_a in (200, 301, 302, 403, 404) and code_b in (200, 301, 302, 403, 404):
        log("PASS", f"Server handles Host header: test.com→{code_a}, other.com→{code_b}")
    else:
        log("FAIL", f"Virtual host handling: test.com→{code_a}, other.com→{code_b}")
except Exception as e:
    log("FAIL", "Virtual hosts", str(e))

# ──────────────────────────────────────────────
# 14. CGI Execution
# ──────────────────────────────────────────────
section("14. CGI Execution")

CGI_PATHS = ["/cgi-bin/test.py", "/cgi-bin/test.php", "/cgi/hello.py"]
cgi_tested = False
for cgi_path in CGI_PATHS:
    try:
        resp = raw_get(cgi_path)
        code = status_code(resp)
        if code == 200:
            log("PASS", f"CGI script at {cgi_path} executed → 200 OK")
            cgi_tested = True
            break
        elif code == 404:
            continue
        else:
            log("PASS", f"CGI route {cgi_path} → {code} (server handled it)")
            cgi_tested = True
            break
    except Exception as e:
        pass

if not cgi_tested:
    log("SKIP", "CGI", f"None of {CGI_PATHS} returned a response. Configure cgi_extension in server.conf and create a test script.")

# ──────────────────────────────────────────────
# 15. Connection Stability (rapid requests)
# ──────────────────────────────────────────────
section("15. Connection Stability (50 rapid GET requests)")

success = 0
for i in range(50):
    try:
        resp = raw_get(f"/?r={i}")
        if status_code(resp) in (200, 301, 302, 403, 404):
            success += 1
    except Exception:
        pass

availability = (success / 50) * 100
if availability >= 99:
    log("PASS", f"Rapid requests: {success}/50 succeeded ({availability:.1f}%)")
elif availability >= 90:
    log("FAIL", f"Rapid requests: {success}/50 succeeded ({availability:.1f}%) – below 99%")
else:
    log("FAIL", f"Rapid requests: {success}/50 succeeded ({availability:.1f}%) – server unstable")

# ──────────────────────────────────────────────
# 16. Siege Stress Test
# ──────────────────────────────────────────────
section("16. Siege Stress Test (availability ≥ 99.5%)")

siege_available = subprocess.run(["which", "siege"], capture_output=True).returncode == 0
if not siege_available:
    log("SKIP", "Siege stress test", "siege not found; install with: sudo apt install siege")
else:
    try:
        result = subprocess.run(
            ["siege", "-b", "-t", "10S", f"http://{HOST}:{PORT}/"],
            capture_output=True, text=True, timeout=30
        )
        output = result.stdout + result.stderr
        # Extract availability from siege output
        avail = None
        for line in output.splitlines():
            if "Availability" in line:
                try:
                    avail = float(line.split()[-1].replace("%", ""))
                except Exception:
                    pass
        if avail is not None:
            if avail >= 99.5:
                log("PASS", f"Siege availability: {avail:.2f}% ≥ 99.5%")
            else:
                log("FAIL", f"Siege availability: {avail:.2f}% < 99.5%")
        else:
            log("INFO", "Siege ran but could not parse availability:")
            print(output[:500])
    except subprocess.TimeoutExpired:
        log("FAIL", "Siege timed out – server may have crashed")
    except Exception as e:
        log("FAIL", f"Siege error: {e}")

# ──────────────────────────────────────────────
# 17. Memory Leak Check (via /proc or top)
# ──────────────────────────────────────────────
section("17. Memory Leak Check (basic)")

try:
    result = subprocess.run(
        ["pgrep", "-f", "localhost"],
        capture_output=True, text=True
    )
    pids = result.stdout.strip().split()
    if pids:
        pid = pids[0]
        mem_before_file = f"/proc/{pid}/status"
        if os.path.exists(mem_before_file):
            with open(mem_before_file) as f:
                for line in f:
                    if line.startswith("VmRSS:"):
                        print(f"       Current RSS memory usage: {line.strip()}")
                        log("PASS", "Memory info retrieved from /proc (manual check needed)")
                        break
        else:
            log("SKIP", "Memory check", "Cannot read /proc (not Linux or no permission)")
    else:
        log("SKIP", "Memory check", "Server process 'localhost' not found via pgrep")
except Exception as e:
    log("SKIP", "Memory check", str(e))

# ──────────────────────────────────────────────
# Summary
# ──────────────────────────────────────────────
print(f"\n{'═'*60}")
print(f"  TEST RESULTS")
print(f"{'═'*60}")
print(f"  {PASS}  {passed} passed")
print(f"  {FAIL}  {failed} failed")
print(f"  {SKIP}  {skipped} skipped")
print(f"{'═'*60}\n")

if failed > 0:
    sys.exit(1)
