#!/usr/bin/env python3
# gen_fixture.py — deterministic access-log fixtures for apps/loglens.
#
# Both fixtures are generated from the LCG below (seed 42); regenerating
# must be byte-identical. The log format is Common Log Format with the
# fixture contract: exactly two '"' per line, canonical integer status
# and bytes (no signs, no leading zeros, '-' never appears as bytes),
# and no CR anywhere. These guarantees are what the four-engine parser
# (operon/python/rust/node) relies on for byte-identity.
#
# Reproducibility contract: generate small THEN big (osid is a GLOBAL
# stream that advances across both generations — same discipline as
# apps/csvstat).
import os

LCG = 42


def nxt():
    global LCG
    LCG = (1103515245 * LCG + 12345) % 2147483648
    return LCG


HOSTS = [
    "10.0.0.7", "10.0.0.12", "10.0.1.33", "10.0.1.90", "10.0.2.4",
    "192.168.3.21", "192.168.3.99", "192.168.7.5", "172.16.0.2",
    "172.16.4.18", "proxy-01.internal", "proxy-02.internal",
    "cache-eu-west", "cache-us-east", "edge-asia-3", "gateway.dmz",
    "monitor.local", "backup.net", "10.1.9.200", "10.2.3.4",
    "192.168.1.1", "172.20.1.7", "cache-eu-west-2", "edge-asia-1",
]

URLS = [
    "/", "/health", "/metrics", "/api/v1/items", "/api/v1/users/42",
    "/api/v1/users/7", "/api/v1/orders", "/api/v1/orders/9182",
    "/api/v1/cart", "/api/v1/session", "/api/v1/search",
    "/api/v2/items", "/api/v2/orders", "/static/app.js",
    "/static/app.css", "/static/vendor.js", "/img/logo.png",
    "/img/banner.jpg", "/img/icons.svg", "/download/report.pdf",
    "/download/invoice.pdf", "/docs/index.html", "/docs/guide.html",
    "/docs/api.md", "/blog/2026/launch", "/blog/2026/update",
    "/login", "/logout", "/signup", "/favicon.ico",
    "/api/v1/items/17", "/api/v1/items/91", "/api/v1/health/deep",
    "/ws/stream", "/upload", "/export/csv", "/export/json",
    "/robots.txt", "/sitemap.xml", "/api/v1/ping",
]

METHODS = ["GET", "GET", "GET", "POST", "HEAD", "PUT"]
PROTOS = ["HTTP/1.1", "HTTP/1.1", "HTTP/1.1", "HTTP/1.0"]
# 17 distinct statuses, heavily 2xx-weighted with a real error tail
STATUSES = [200, 200, 200, 200, 200, 200, 204, 301, 302,
            400, 401, 403, 404, 404, 500, 502, 503]

MONTH_DAY = "04/Oct/2026"


def gen_line():
    host = HOSTS[nxt() % len(HOSTS)]
    url = URLS[nxt() % len(URLS)]
    method = METHODS[nxt() % len(METHODS)]
    proto = PROTOS[nxt() % len(PROTOS)]
    status = STATUSES[nxt() % len(STATUSES)]
    lo = nxt() % 60000
    if status >= 400:
        lo = lo % 800          # error bodies are small
    if status in (204, 301, 302):
        lo = 0                 # redirects/204 carry no body
    sec = nxt() % 86400
    hh = sec // 3600
    mm = (sec % 3600) // 60
    ss = sec % 60
    return (f"{host} - - [{MONTH_DAY}:{hh:02d}:{mm:02d}:{ss:02d} +0000] "
            f"\"{method} {url} {proto}\" {status} {lo}")


def gen_file(path, n):
    lines = [gen_line() for _ in range(n)]
    with open(path, "w") as f:
        f.write("\n".join(lines) + "\n")
    return path


def main():
    here = os.path.dirname(os.path.abspath(__file__))
    data = os.path.join(here, "data")
    os.makedirs(data, exist_ok=True)
    # reproducibility contract: small THEN big (global LCG stream)
    gen_file(os.path.join(data, "small.log"), 150)
    gen_file(os.path.join(data, "big.log"), 50000)
    print("fixtures written to", data)


if __name__ == "__main__":
    main()
