#!/usr/bin/env python3
"""Record deterministic golden behavioral cases for Waves 3-6 from NousResearch/hermes-agent.

Always runs against an isolated temporary HERMES_HOME so ~/.hermes is never modified.
"""

from __future__ import annotations

import json
import os
import sys
import types
from pathlib import Path
from typing import Any

# Prevent pm bootstrap from re-execing into an unprovisioned temp HERMES_HOME.
_hb_stub = types.ModuleType("hermes_bootstrap")
_hb_stub._happy_eyeballs_create_connection = lambda *a, **kw: None  # type: ignore[attr-defined]
sys.modules["hermes_bootstrap"] = _hb_stub

REAL_HERMES = Path.home() / ".hermes"
env_home = os.environ.get("HERMES_HOME", "").strip()
if not env_home or Path(env_home).resolve() == REAL_HERMES.resolve():
    raise SystemExit("Refusing to run oracle recorder without an isolated temp HERMES_HOME.")

HERMES_SRC = Path("/home/ridlo/.hermes/hermes-agent")
if str(HERMES_SRC) not in sys.path:
    sys.path.insert(0, str(HERMES_SRC))

setattr(sys, "_hermes_relaunched", True)
os.environ.setdefault("HERMES_SKIP_PM_BOOTSTRAP", "1")

OUT_DIR = Path("/home/ridlo/github/saif-agent/conformance/cases")
OUT_DIR.mkdir(parents=True, exist_ok=True)


def write_jsonl(name: str, rows: list[dict[str, Any]]) -> None:
    path = OUT_DIR / name
    with path.open("w", encoding="utf-8") as fh:
        for row in rows:
            fh.write(json.dumps(row, ensure_ascii=False) + "\n")
    print(f"Wrote {len(rows):3d} cases -> {path}")


def record_codex_transport() -> None:
    from agent.transports.codex import (
        _alias_reserved_tools,
        _bounded_prompt_cache_key,
        _cache_scope_from_session_id,
    )

    rows: list[dict[str, Any]] = []
    for raw in [
        None,
        "",
        "   ",
        "sess_short_123",
        "a" * 64,
        "b" * 65,
        "agent:profile_alpha:telegram:12345678901234567890123456789012345678901234567890",
    ]:
        rows.append(
            {
                "op": "bounded_prompt_cache_key",
                "input": raw,
                "expected": _bounded_prompt_cache_key(raw),
            }
        )

    for sid in [
        "cron_daily_digest_20261120_090000",
        "cron_backup_01_20260101_235959",
        "sess_regular_99",
        "agent:main:cli:interactive",
    ]:
        rows.append(
            {
                "op": "cache_scope_from_session_id",
                "session_id": sid,
                "expected": _cache_scope_from_session_id(sid),
            }
        )

    sample_tools = [
        {"type": "function", "name": "read_file"},
        {"type": "function", "name": "tool_search"},
        {"type": "function", "name": "web_search"},
        {"type": "function", "name": "search_files"},
    ]
    for reserved in [
        ("tool_search",),
        ("web_search", "search_files"),
        ("tool_search", "web_search"),
    ]:
        rewritten, alias_map = _alias_reserved_tools(sample_tools, reserved)
        rows.append(
            {
                "op": "alias_reserved_tools",
                "tools": [t["name"] for t in sample_tools],
                "reserved": list(reserved),
                "expected_tools": [t["name"] for t in rewritten],
                "expected_alias_map": alias_map,
            }
        )

    write_jsonl("codex_transport.jsonl", rows)


def record_batch_sharegpt() -> None:
    import json as _json

    # Call batch_runner._tool_call_succeeded directly (stub optional rich/fire if missing)
    try:
        from batch_runner import _tool_call_succeeded
    except ModuleNotFoundError:
        # Minimal stub only for optional UI libs if venv doesn't have one
        for mod in ("fire", "rich", "rich.console", "rich.progress"):
            if mod not in sys.modules:
                m = types.ModuleType(mod)
                for attr in (
                    "Console",
                    "Progress",
                    "BarColumn",
                    "MofNCompleteColumn",
                    "SpinnerColumn",
                    "TextColumn",
                    "TimeRemainingColumn",
                ):
                    setattr(m, attr, object)
                sys.modules[mod] = m
        from batch_runner import _tool_call_succeeded

    rows: list[dict[str, Any]] = []
    cases = [
        ("json_success_true", _json.dumps({"success": True, "output": "ok"})),
        ("json_success_false", _json.dumps({"success": False})),
        ("json_error_string", _json.dumps({"error": "boom"})),
        ("json_error_null_nonzero_exit", _json.dumps({"error": None, "exit_code": 1})),
        ("json_nested_content_error", _json.dumps({"content": {"error": "bad"}})),
        ("plain_ok", "completed cleanly"),
        ("plain_error_prefix", "Error: failed to read"),
        ("plain_error_whitespace", "  error: permission denied"),
        ("empty_str", ""),
    ]
    for name, payload in cases:
        rows.append(
            {
                "op": "tool_call_succeeded",
                "name": name,
                "content": payload,
                "expected": bool(_tool_call_succeeded(payload)),
            }
        )
    write_jsonl("batch_sharegpt.jsonl", rows)


def record_media_exec_helpers() -> None:
    from tools.code_execution_tool import MAX_STDOUT_BYTES, _truncate_stdout_text
    from tools.tts_tool_providers import _config_bool, _tts_response_format_from_path

    rows: list[dict[str, Any]] = []
    for p in ["out/audio.ogg", "out/audio.wav", "out/audio.flac", "out/audio.mp3", "out/audio.bin"]:
        rows.append(
            {
                "op": "tts_response_format_from_path",
                "path": p,
                "expected": _tts_response_format_from_path(p),
            }
        )

    for raw, default in [
        ("1", False),
        ("true", False),
        ("YES", False),
        ("on", False),
        ("enabled", False),
        ("0", True),
        ("false", True),
        ("off", True),
        ("disabled", True),
        ("random_unrecognized", True),
        ("random_unrecognized", False),
    ]:
        rows.append(
            {
                "op": "config_bool",
                "raw": raw,
                "default": default,
                "expected": bool(_config_bool(raw, default)),
            }
        )

    for label, text in [
        ("under_cap", "hello world\n" * 10),
        ("over_cap_70k", "A" * 70_000),
    ]:
        truncated_text, meta = _truncate_stdout_text(text)
        rows.append(
            {
                "op": "truncate_stdout_text",
                "name": label,
                "input_bytes": len(text.encode("utf-8")),
                "max_stdout_bytes": MAX_STDOUT_BYTES,
                "expected_truncated": bool(meta["stdout_truncated"]),
                "expected_bytes_captured": int(meta["stdout_bytes_captured"]),
                "expected_bytes_total": int(meta["stdout_bytes_total"]),
                "expected_bytes_omitted": int(meta["stdout_bytes_omitted"]),
                "has_truncation_marker": "truncated" in truncated_text.lower(),
            }
        )

    write_jsonl("media_exec_helpers.jsonl", rows)


if __name__ == "__main__":
    record_codex_transport()
    record_batch_sharegpt()
    record_media_exec_helpers()
