#!/usr/bin/env python3
import os
import sys

print("Content-Type: text/html")
print("Status: 200 OK")
print("")
print("<html><body>")
print("<h1>CGI Test Success!</h1>")
print(f"<p>REQUEST_METHOD: {os.environ.get('REQUEST_METHOD', 'N/A')}</p>")
print(f"<p>PATH_INFO: {os.environ.get('PATH_INFO', 'N/A')}</p>")
print(f"<p>QUERY_STRING: {os.environ.get('QUERY_STRING', 'N/A')}</p>")

if os.environ.get('REQUEST_METHOD') == 'POST':
    body = sys.stdin.read()
    print(f"<p>Body content length: {len(body)}</p>")
    print(f"<pre>{body}</pre>")

print("</body></html>")
