#!/usr/bin/env python3
"""Take a read-only snapshot of a saved Zed agent thread for local replay."""

import argparse
import datetime
import hashlib
import json
import os
from pathlib import Path
import shutil
import sqlite3
import subprocess
import sys


def default_database():
    if sys.platform == "darwin":
        data = Path.home() / "Library/Application Support/Zed"
    elif sys.platform == "win32":
        local_app_data = os.environ.get("LOCALAPPDATA")
        if not local_app_data:
            raise ValueError("LOCALAPPDATA is unavailable; pass --database explicitly")
        data = Path(local_app_data) / "Zed"
    else:
        data = Path(os.environ.get("XDG_DATA_HOME", Path.home() / ".local/share")) / "zed"
    return data / "threads/threads.db"


def open_database(path):
    path = path.expanduser().resolve(strict=True)
    connection = sqlite3.connect(path.as_uri() + "?mode=ro", uri=True, timeout=5)
    connection.execute("PRAGMA query_only=ON")
    return connection


def list_threads(connection, limit):
    return [
        {"id": row[0], "title": row[1], "updated_at": row[2], "encoding": row[3], "stored_bytes": row[4]}
        for row in connection.execute(
            "SELECT id, summary, updated_at, data_type, length(data) "
            "FROM threads ORDER BY updated_at DESC LIMIT ?", (limit,)
        )
    ]


def read_thread(connection, thread_id):
    row = connection.execute(
        "SELECT summary, updated_at, data_type, data FROM threads WHERE id = ?", (thread_id,)
    ).fetchone()
    if row is None:
        raise ValueError("No saved thread matches the supplied id")
    title, updated_at, encoding, stored = row
    if encoding == "json":
        decoded = stored.encode("utf-8") if isinstance(stored, str) else stored
    elif encoding == "zstd":
        executable = shutil.which("zstd")
        if executable is None:
            raise ValueError("This thread is zstd-compressed. Install the zstd CLI to export it.")
        result = subprocess.run(
            [executable, "--decompress", "--quiet", "--stdout"],
            input=stored, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            check=False, timeout=30,
        )
        if result.returncode:
            raise ValueError("zstd could not decompress the saved thread: " + result.stderr.decode("utf-8", errors="replace"))
        decoded = result.stdout
    else:
        raise ValueError("Unsupported saved-thread encoding: " + str(encoding))
    payload = json.loads(decoded)
    if not isinstance(payload, dict) or not isinstance(payload.get("messages"), list):
        raise ValueError("Saved data is not a supported agent thread object with a messages list")
    metadata = {
        "thread_id": thread_id,
        "title": title,
        "updated_at": updated_at,
        "snapshot_at": datetime.datetime.now(datetime.timezone.utc).isoformat(),
        "encoding": encoding,
        "stored_bytes": len(stored),
        "decoded_bytes": len(decoded),
        "source_sha256": hashlib.sha256(decoded).hexdigest(),
        "saved_messages": len(payload["messages"]),
        "version": payload.get("version"),
        "database_access": "read-only",
    }
    payload["_cache_replay_export"] = metadata
    return payload, metadata


def write_snapshot(path, payload):
    path = path.expanduser()
    if not path.parent.is_dir():
        raise ValueError("Output parent directory must already exist: " + str(path.parent))
    # Never overwrite an existing snapshot or expose its contents via default permissions.
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
    try:
        with os.fdopen(descriptor, "w", encoding="utf-8") as output:
            json.dump(payload, output, ensure_ascii=False, separators=(",", ":"))
            output.write("\n")
            output.flush()
            os.fsync(output.fileno())
    except BaseException:
        try:
            path.unlink()
        except OSError as cleanup_error:
            print("Could not remove incomplete snapshot: " + str(cleanup_error), file=sys.stderr)
        raise


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--database", type=Path, help="threads.db location; defaults to Zed's platform data directory")
    parser.add_argument("--list", action="store_true", help="Print recent thread metadata only; never message text")
    parser.add_argument("--limit", type=int, default=10)
    parser.add_argument("--thread-id", help="Permanent id from --list")
    parser.add_argument("--output", type=Path, help="New private JSON snapshot file; contains conversation data")
    args = parser.parse_args()
    if args.limit < 1:
        parser.error("--limit must be positive")
    if args.list and (args.thread_id or args.output):
        parser.error("--list cannot be combined with --thread-id or --output")
    if not args.list and (not args.thread_id or not args.output):
        parser.error("Specify --list, or both --thread-id and --output")
    database = args.database or default_database()
    connection = open_database(database)
    try:
        if args.list:
            print(json.dumps(list_threads(connection, args.limit), indent=2))
        else:
            payload, metadata = read_thread(connection, args.thread_id)
            write_snapshot(args.output, payload)
            print(json.dumps({"output": str(args.output), **metadata}, indent=2))
    finally:
        connection.close()


if __name__ == "__main__":
    try:
        main()
    except (OSError, ValueError, sqlite3.Error, subprocess.SubprocessError) as error:
        print("Thread export failed: " + str(error), file=sys.stderr)
        sys.exit(1)
