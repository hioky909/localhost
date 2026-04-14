#!/usr/bin/env python3
import os, sys

method = os.environ.get('REQUEST_METHOD', 'GET')
path_info = os.environ.get('PATH_INFO', '/')

print("Content-Type: text/html")
print()
print(f"<html><body><h1>CGI Hello</h1><p>Method: {method}</p><p>Path: {path_info}</p></body></html>")
