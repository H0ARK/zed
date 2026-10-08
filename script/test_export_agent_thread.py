import importlib.util
import json
import sqlite3
import subprocess
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

spec = importlib.util.spec_from_file_location(
    "export_agent_thread", Path(__file__).with_name("export-agent-thread.py")
)
exporter = importlib.util.module_from_spec(spec)
spec.loader.exec_module(exporter)


class ExportAgentThreadTests(unittest.TestCase):
    def setUp(self):
        self.directory = tempfile.TemporaryDirectory()
        self.addCleanup(self.directory.cleanup)
        self.root = Path(self.directory.name)
        self.database = self.root / "threads.db"
        self.payload = {
            "version": "0.3.0",
            "title": "fixture",
            "updated_at": "2026-10-08T00:00:00Z",
            "messages": [{"User": {"id": "user", "content": [{"Text": "private 🦀"}]}}],
        }
        with sqlite3.connect(self.database) as connection:
            connection.execute(
                "CREATE TABLE threads (id TEXT PRIMARY KEY, summary TEXT, updated_at TEXT, data_type TEXT, data BLOB)"
            )
            connection.execute(
                "INSERT INTO threads VALUES (?, ?, ?, ?, ?)",
                (
                    "fixture-id",
                    "fixture",
                    self.payload["updated_at"],
                    "json",
                    json.dumps(self.payload).encode(),
                ),
            )

    def connection(self):
        connection = exporter.open_database(self.database)
        self.addCleanup(connection.close)
        return connection

    def test_read_only_export_does_not_change_database(self):
        before = self.database.read_bytes()
        connection = self.connection()
        with self.assertRaises(sqlite3.OperationalError):
            connection.execute("DELETE FROM threads")
        payload, metadata = exporter.read_thread(connection, "fixture-id")
        self.assertEqual(payload["messages"], self.payload["messages"])
        self.assertEqual(metadata["saved_messages"], 1)
        self.assertEqual(metadata["database_access"], "read-only")
        self.assertEqual(self.database.read_bytes(), before)

    def test_listing_exposes_metadata_not_conversation(self):
        listing = exporter.list_threads(self.connection(), 1)
        self.assertEqual(listing[0]["id"], "fixture-id")
        self.assertNotIn("private", json.dumps(listing))
        self.assertNotIn("messages", listing[0])

    def test_snapshot_is_private_and_never_overwritten(self):
        payload, _ = exporter.read_thread(self.connection(), "fixture-id")
        output = self.root / "snapshot.json"
        exporter.write_snapshot(output, payload)
        self.assertEqual(
            json.loads(output.read_text())["messages"], self.payload["messages"]
        )
        if exporter.os.name == "posix":
            self.assertEqual(output.stat().st_mode & 0o777, 0o600)
        before = output.read_bytes()
        with self.assertRaises(FileExistsError):
            exporter.write_snapshot(output, {"messages": []})
        self.assertEqual(output.read_bytes(), before)

    def test_missing_id_is_parameterized_and_does_not_change_rows(self):
        connection = self.connection()
        with self.assertRaisesRegex(ValueError, "No saved thread"):
            exporter.read_thread(connection, "fixture-id'; DELETE FROM threads; --")
        self.assertEqual(len(exporter.list_threads(connection, 10)), 1)

    def test_missing_database_is_not_created(self):
        missing = self.root / "missing.db"
        with self.assertRaises(FileNotFoundError):
            exporter.open_database(missing)
        self.assertFalse(missing.exists())

    def test_zstd_uses_stdin_and_decompresses_without_shell(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute(
                "UPDATE threads SET data_type='zstd', data=?", (b"compressed",)
            )
        result = subprocess.CompletedProcess(
            [], 0, json.dumps(self.payload).encode(), b""
        )
        with (
            patch.object(exporter.shutil, "which", return_value="/fixture/zstd"),
            patch.object(exporter.subprocess, "run", return_value=result) as run,
        ):
            payload, _ = exporter.read_thread(self.connection(), "fixture-id")
        self.assertEqual(payload["messages"], self.payload["messages"])
        self.assertEqual(run.call_args.kwargs["input"], b"compressed")
        self.assertNotIn("shell", run.call_args.kwargs)
        self.assertEqual(run.call_args.kwargs["timeout"], 30)

    def test_missing_zstd_and_unsupported_encoding_fail_explicitly(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE threads SET data_type='zstd'")
        with patch.object(exporter.shutil, "which", return_value=None):
            with self.assertRaisesRegex(ValueError, "Install the zstd CLI"):
                exporter.read_thread(self.connection(), "fixture-id")
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE threads SET data_type='unknown'")
        with self.assertRaisesRegex(ValueError, "Unsupported saved-thread encoding"):
            exporter.read_thread(self.connection(), "fixture-id")

    def test_modern_metadata_and_content_are_preserved_without_unknown_field_omission(self):
        self.payload.update({
            "infinite_context": True,
            "memory_archived": True,
            "memory_turn_start": [1, 2],
            "cumulative_token_usage": {"input_tokens": 5, "cache_creation_input_tokens": 3},
            "future_metadata": {"must_not_disappear": "private"},
        })
        self.payload["messages"].extend([
            "Resume",
            {"Compaction": {"Summary": "private summary"}},
            {"User": {"id": "mention", "content": [{"Mention": {
                "uri": {"File": {"abs_path": "/not-read/file.rs"}},
                "content": "saved content",
            }}]}},
        ])
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE threads SET data=?", (json.dumps(self.payload).encode(),))
        payload, metadata = exporter.read_thread(self.connection(), "fixture-id")
        for field, value in self.payload.items():
            self.assertEqual(payload[field], value)
        output = self.root / "modern.json"
        exporter.write_snapshot(output, payload)
        self.assertEqual(json.loads(output.read_text()), payload)
        self.assertEqual(metadata["saved_messages"], 4)

    def test_zstd_failure_does_not_echo_transcript_or_decoder_stderr(self):
        with sqlite3.connect(self.database) as connection:
            connection.execute("UPDATE threads SET data_type='zstd', data=?", (b"compressed",))
        result = subprocess.CompletedProcess([], 1, b"", b"PRIVATE_TRANSCRIPT")
        with (
            patch.object(exporter.shutil, "which", return_value="/fixture/zstd"),
            patch.object(exporter.subprocess, "run", return_value=result),
        ):
            with self.assertRaisesRegex(ValueError, "^zstd could not decompress the saved thread$"):
                exporter.read_thread(self.connection(), "fixture-id")

    def test_bad_payload_does_not_leave_a_partial_snapshot(self):
        output = self.root / "bad.json"
        with self.assertRaises(TypeError):
            exporter.write_snapshot(output, {"unsupported": object()})
        self.assertFalse(output.exists())


if __name__ == "__main__":
    unittest.main()
