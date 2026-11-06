#!/usr/bin/env python3
"""Manual check: a COMMIT whose acknowledgment never reaches the client.

Puts a small PostgreSQL protocol proxy between the outbox and the database. The
proxy forwards a `COMMIT` to the server, waits until the server has answered, and
then drops both connections *without* relaying the answer. The server has
committed; the client only sees a dead connection. Two scenarios are checked:

* pooled recording reports `DatabaseUncertain`, and a same-ID retry then adds no
  rows while the committed rows remain retained;
* a caller-owned business-plus-outbox transaction gets an application commit
  error (never a recording error), and both the business row and the outbox rows
  are in fact committed.

Rows are observed over a separate direct connection. Needs
OUTBOX_TEST_DATABASE_URL (a disposable PostgreSQL) and psql. The result is printed
as a report to be recorded on the delivering pull request; closing the check is
the repository owner's call.
"""
import os
import pathlib
import re
import socket
import subprocess
import sys
import threading
import urllib.parse
import uuid

ROOT = pathlib.Path(__file__).resolve().parent.parent
MANIFEST = str(ROOT / "libs" / "Cargo.toml")


def recv_exact(sock, n):
    data = b""
    while len(data) < n:
        chunk = sock.recv(n - len(data))
        if not chunk:
            raise EOFError
        data += chunk
    return data


def close(*socks):
    for s in socks:
        try:
            s.shutdown(socket.SHUT_RDWR)
        except OSError:
            pass
        s.close()


class Session:
    """One proxied connection. Assumes `sslmode=disable` (no SSLRequest)."""

    def __init__(self, client, upstream):
        self.client, self.server = client, socket.create_connection(upstream)
        self.commit_seen = threading.Event()
        self.cuts = 0
        threading.Thread(target=self.upstream, daemon=True).start()
        threading.Thread(target=self.downstream, daemon=True).start()

    def upstream(self):
        try:
            length = int.from_bytes(recv_exact(self.client, 4), "big")  # startup: no type byte
            self.server.sendall(length.to_bytes(4, "big") + recv_exact(self.client, length - 4))
            while True:
                kind = recv_exact(self.client, 1)
                length = int.from_bytes(recv_exact(self.client, 4), "big")
                body = recv_exact(self.client, length - 4)
                if kind == b"Q" and body.upper().startswith(b"COMMIT"):
                    self.commit_seen.set()
                self.server.sendall(kind + length.to_bytes(4, "big") + body)
        except (EOFError, OSError):
            close(self.client, self.server)

    def downstream(self):
        try:
            while True:
                kind = recv_exact(self.server, 1)
                length = int.from_bytes(recv_exact(self.server, 4), "big")
                body = recv_exact(self.server, length - 4)
                if self.commit_seen.is_set():
                    if kind == b"Z":  # ReadyForQuery: the commit is done; withhold it all
                        print("proxy: COMMIT executed by the server; acknowledgment withheld", file=sys.stderr)
                        close(self.client, self.server)
                        return
                    continue
                self.client.sendall(kind + length.to_bytes(4, "big") + body)
        except (EOFError, OSError):
            close(self.client, self.server)


def serve(listener, upstream):
    while True:
        try:
            client, _ = listener.accept()
        except OSError:
            return
        Session(client, upstream)


def psql(url, *args):
    return subprocess.run(["psql", url, "-q", "-v", "ON_ERROR_STOP=1", *args], check=True, capture_output=True, text=True)


def main():
    base = os.environ.get("OUTBOX_TEST_DATABASE_URL") or sys.exit("OUTBOX_TEST_DATABASE_URL is not set")
    parsed = urllib.parse.urlsplit(base)
    upstream = (parsed.hostname, parsed.port or 5432)

    schema = "c_" + uuid.uuid4().hex
    options = f"options=-csearch_path%3D{schema}&sslmode=disable"
    psql(base, "-c", f"CREATE SCHEMA {schema}")
    try:
        env_schema = dict(os.environ, PGOPTIONS=f"-csearch_path={schema}")
        subprocess.run(["psql", base, "-q", "-f", str(ROOT / "docs" / "outbox_entries.sql"),
                        "-c", "CREATE TABLE business (n INT)"], check=True, capture_output=True, env=env_schema)

        listener = socket.socket()
        listener.bind(("127.0.0.1", 0))
        listener.listen()
        threading.Thread(target=serve, args=(listener, upstream), daemon=True).start()

        def url(host, port):
            netloc = f"{parsed.username}:{parsed.password}@{host}:{port}"
            return urllib.parse.urlunsplit((parsed.scheme, netloc, parsed.path, options, ""))

        env = dict(os.environ,
                   PROXY_DATABASE_URL=url("127.0.0.1", listener.getsockname()[1]),
                   DIRECT_DATABASE_URL=url(upstream[0], upstream[1]))
        run = subprocess.run(["cargo", "run", "--quiet", "--manifest-path", MANIFEST, "-p", "mulac-outbox",
                              "--example", "commit_cut_check"], capture_output=True, text=True, env=env)
        sys.stderr.write(run.stderr)
        if run.returncode != 0:
            sys.exit(f"helper failed:\n{run.stdout}")
        results = {}
        for line in run.stdout.splitlines():
            if line.startswith("RESULT "):
                _, name, *pairs = line.split()
                results[name] = dict(p.split("=") for p in pairs)
        print(run.stdout)

        pooled, caller = results["pooled"], results["caller"]
        checks = [
            ("pooled recording reports DatabaseUncertain", pooled["error"] == "DatabaseUncertain"
             and pooled["outcome"] == "Uncertain"),
            ("pooled rows were really committed", pooled["rows_committed"] == "3"),
            ("same-ID retry succeeds and adds no rows", pooled["retry_ok"] == "true"
             and pooled["rows_after_retry"] == "3"),
            ("caller's COMMIT failed with an application error", caller["commit_failed"] == "true"),
            ("business row and outbox rows were really committed",
             caller["business_rows"] == "1" and caller["outbox_rows_committed"] == "2"),
            ("same-ID retry adds no rows", caller["retry_ok"] == "true" and caller["rows_after_retry"] == "2"),
        ]
        failed = False
        for name, ok in checks:
            print(("PASS  " if ok else "FAIL  ") + name)
            failed |= not ok
        sys.exit(1 if failed else 0)
    finally:
        psql(base, "-c", f"DROP SCHEMA {schema} CASCADE")


if __name__ == "__main__":
    main()
