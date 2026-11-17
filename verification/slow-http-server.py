#!/usr/bin/env python3
# SPDX-License-Identifier: GPL-3.0-or-later
"""A static file server on 127.0.0.1 that sends files slowly, so a window
test can capture an update while it downloads.

Usage: slow-http-server.py PORT DIR [SECONDS_PER_64KIB]   (default 0.4)
"""
import functools
import http.server
import sys
import time

port, root = int(sys.argv[1]), sys.argv[2]
delay = float(sys.argv[3]) if len(sys.argv) > 3 else 0.4


class Slow(http.server.SimpleHTTPRequestHandler):
    def copyfile(self, source, outputfile):
        while chunk := source.read(64 * 1024):
            outputfile.write(chunk)
            outputfile.flush()
            time.sleep(delay)


handler = functools.partial(Slow, directory=root)
http.server.ThreadingHTTPServer(("127.0.0.1", port), handler).serve_forever()
