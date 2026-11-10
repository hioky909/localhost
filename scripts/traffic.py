#!/usr/bin/env python3
"""Génère du trafic HTTP réaliste contre un serveur localhost (bibliothèque standard uniquement).

Mélange : pages, listing, CGI GET/POST, uploads puis suppressions, redirections, 404, 405,
méthode inconnue (501) et corps trop gros (413), avec des rafales concurrentes.

    python3 scripts/traffic.py                # 127.0.0.1:8080, 30 secondes
    python3 scripts/traffic.py 127.0.0.1 8080 --duration 60 --workers 8
"""
import argparse
import http.client
import random
import threading
import time

ap = argparse.ArgumentParser()
ap.add_argument("host", nargs="?", default="127.0.0.1")
ap.add_argument("port", nargs="?", type=int, default=8080)
ap.add_argument("--duration", type=float, default=30)
ap.add_argument("--workers", type=int, default=4)
ap.add_argument("--pace", type=float, default=0.25, help="pause moyenne entre requêtes (s)")
args = ap.parse_args()

PAGES = ["/", "/", "/", "/index.html", "/upload/", "/errors/404.html"]
MISSING = ["/favicon.ico", "/wp-login.php", "/api/v2/users", "/.env", "/robots.txt"]
counts = {}
lock = threading.Lock()


def req(method, path, body=None, headers=None):
    conn = http.client.HTTPConnection(args.host, args.port, timeout=5)
    try:
        conn.request(method, path, body=body, headers=headers or {})
        r = conn.getresponse()
        r.read()
        with lock:
            counts[r.status] = counts.get(r.status, 0) + 1
        return r.status
    except Exception:
        with lock:
            counts["erreur"] = counts.get("erreur", 0) + 1
    finally:
        conn.close()


def upload_then_delete():
    name = f"note-{random.randint(1000, 9999)}.txt"
    boundary = "----localhost"
    data = f"rapport du {time.strftime('%H:%M:%S')}\n".encode() * random.randint(1, 40)
    body = (
        f"--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"{name}\"\r\n"
        f"Content-Type: text/plain\r\n\r\n"
    ).encode() + data + f"\r\n--{boundary}--\r\n".encode()
    req("POST", "/upload", body, {"Content-Type": f"multipart/form-data; boundary={boundary}"})
    time.sleep(random.uniform(0.2, 1.0))
    req("DELETE", f"/upload/{name}")


def one():
    r = random.random()
    if r < 0.50:
        req("GET", random.choice(PAGES))
    elif r < 0.62:
        req("GET", f"/cgi-bin/test.py?user={random.choice(['lea', 'hugo', 'ines', 'yanis'])}&page={random.randint(1, 9)}")
    elif r < 0.68:
        req("POST", "/cgi-bin/test.py", b"message=bonjour&source=traffic", {"Content-Type": "application/x-www-form-urlencoded"})
    elif r < 0.76:
        upload_then_delete()
    elif r < 0.84:
        req("GET", random.choice(MISSING))
    elif r < 0.89:
        req("GET", "/redirect_test")
    elif r < 0.93:
        req("DELETE", "/")
    elif r < 0.96:
        req("PATCH", "/")
    else:
        req("POST", "/", b"x" * (2 * 1024 * 1024))  # > client_max_body_size → 413


def worker(stop):
    while time.time() < stop:
        if random.random() < 0.08:  # rafale
            for _ in range(random.randint(5, 20)):
                req("GET", random.choice(PAGES))
        else:
            one()
        time.sleep(random.expovariate(1 / args.pace))


stop = time.time() + args.duration
threads = [threading.Thread(target=worker, args=(stop,)) for _ in range(args.workers)]
for t in threads:
    t.start()
for t in threads:
    t.join()
total = sum(v for v in counts.values())
print(f"{total} requêtes : " + ", ".join(f"{k} × {v}" for k, v in sorted(counts.items(), key=lambda kv: str(kv[0]))))
