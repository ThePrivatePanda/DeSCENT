#!/usr/bin/env python3
# Serves web/ straight from disk and hands /api to a running app, so a page edit shows
# on reload with no cargo build (the app embeds web/ when it is compiled).
#
#   tools/devserve.py 8792 8791      page on 8792, app already running on 8791
#
# Only for working on the page. Sockets and streaming bodies are passed through as is.
import http.client
import http.server
import os
import sys

WEB = os.path.join(os.path.dirname(os.path.abspath(__file__)), '..', 'web')
PAGE_PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 8792
APP_PORT = int(sys.argv[2]) if len(sys.argv) > 2 else 8791


class Handler(http.server.SimpleHTTPRequestHandler):
    def __init__(self, *a, **kw):
        super().__init__(*a, directory=WEB, **kw)

    def log_message(self, *a):
        pass

    def proxy(self):
        length = int(self.headers.get('Content-Length') or 0)
        body = self.rfile.read(length) if length else None
        c = http.client.HTTPConnection('127.0.0.1', APP_PORT, timeout=600)
        try:
            c.request(self.command, self.path, body=body, headers=dict(self.headers))
            r = c.getresponse()
        except OSError as e:
            self.send_error(502, 'app on %d: %s' % (APP_PORT, e))
            return
        self.send_response(r.status)
        for k, v in r.getheaders():
            if k.lower() not in ('transfer-encoding', 'connection'):
                self.send_header(k, v)
        self.end_headers()
        while True:
            chunk = r.read(65536)
            if not chunk:
                break
            self.wfile.write(chunk)
            self.wfile.flush()

    def do_GET(self):
        if self.path.startswith('/api/'):
            return self.proxy()
        return super().do_GET()

    def end_headers(self):
        if not self.path.startswith('/api/'):
            self.send_header('Cache-Control', 'no-store')
        super().end_headers()

    do_POST = proxy
    do_PUT = proxy
    do_DELETE = proxy


if __name__ == '__main__':
    print('page http://127.0.0.1:%d  api -> 127.0.0.1:%d  web %s' % (PAGE_PORT, APP_PORT, os.path.abspath(WEB)))
    http.server.ThreadingHTTPServer(('127.0.0.1', PAGE_PORT), Handler).serve_forever()
