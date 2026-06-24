#!/usr/bin/env python3
"""
Interactive OpenAI-backed memory chat test.

Normal input:
    Ingest the line as an event and show memory changes.

Question input:
    Prefix the line with "提问" to retrieve memories and ask OpenAI.

Examples:
    你> 我最近想买海景房，但是预算只有 300 万
    你> 最近中彩票了，买房预算提高到 800 万
    你> 提问 我买房时有什么偏好和限制？

Run:
    python3 tests/openai_interactive_chat.py --base-url http://localhost:18081

Dependencies:
    pip install requests loguru
"""

from __future__ import annotations

import argparse
import os
import sys
import time
import uuid
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import requests

try:
    from loguru import logger
except ImportError as exc:
    print("Missing dependency: pip install loguru", file=sys.stderr)
    raise SystemExit(1) from exc


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = REPO_ROOT / "memory-server" / "config" / "config.yaml.example"


@dataclass
class OpenAIConfig:
    endpoint: str
    api_key: str
    model: str


@dataclass
class SessionIdentity:
    profile_id: str
    owner_id: str
    scope_id: str


def setup_logger() -> None:
    logger.remove()
    logger.add(
        sys.stderr,
        colorize=True,
        format=(
            "<green>{time:HH:mm:ss}</green> | "
            "<level>{level: <8}</level> | "
            "<cyan>{message}</cyan>"
        ),
    )


def remove_inline_comment(value: str) -> str:
    in_quote: str | None = None
    escaped = False
    chars: list[str] = []
    for ch in value:
        if escaped:
            chars.append(ch)
            escaped = False
            continue
        if ch == "\\":
            chars.append(ch)
            escaped = True
            continue
        if ch in {"'", '"'}:
            if in_quote == ch:
                in_quote = None
            elif in_quote is None:
                in_quote = ch
            chars.append(ch)
            continue
        if ch == "#" and in_quote is None:
            break
        chars.append(ch)
    return "".join(chars).strip()


def parse_scalar(value: str) -> Any:
    value = remove_inline_comment(value).strip()
    if value == "":
        return ""
    if value in {"true", "false"}:
        return value == "true"
    if (value.startswith('"') and value.endswith('"')) or (
        value.startswith("'") and value.endswith("'")
    ):
        return value[1:-1]
    try:
        return int(value)
    except ValueError:
        pass
    try:
        return float(value)
    except ValueError:
        return value


def parse_simple_yaml(path: Path) -> dict[str, Any]:
    data: dict[str, Any] = {}
    stack: list[tuple[int, dict[str, Any]]] = [(-1, data)]

    for raw_line in path.read_text(encoding="utf-8").splitlines():
        if not raw_line.strip() or raw_line.lstrip().startswith("#"):
            continue

        indent = len(raw_line) - len(raw_line.lstrip(" "))
        line = raw_line.strip()
        if ":" not in line:
            continue

        key, raw_value = line.split(":", 1)
        key = key.strip()
        raw_value = raw_value.strip()

        while stack and indent <= stack[-1][0]:
            stack.pop()

        parent = stack[-1][1]
        if raw_value == "":
            child: dict[str, Any] = {}
            parent[key] = child
            stack.append((indent, child))
        else:
            parent[key] = parse_scalar(raw_value)

    return data


def load_yaml(path: Path) -> dict[str, Any]:
    if not path.exists():
        raise FileNotFoundError(f"Config file not found: {path}")

    try:
        import yaml  # type: ignore
    except ImportError:
        return parse_simple_yaml(path)

    loaded = yaml.safe_load(path.read_text(encoding="utf-8"))
    if not isinstance(loaded, dict):
        return {}
    return loaded


def nested_get(data: dict[str, Any], path: str, default: Any = None) -> Any:
    current: Any = data
    for key in path.split("."):
        if not isinstance(current, dict) or key not in current:
            return default
        current = current[key]
    return current


def load_openai_config(config_path: Path) -> OpenAIConfig:
    data = load_yaml(config_path)
    endpoint = (
        os.getenv("OPENAI_BASE_URL")
        or os.getenv("MEMORY_SERVER__LLM__ENDPOINT")
        or nested_get(data, "llm.endpoint")
        or "https://api.openai.com/v1"
    )
    model = (
        os.getenv("OPENAI_MODEL")
        or os.getenv("MEMORY_SERVER__LLM__MODEL")
        or nested_get(data, "llm.model")
        or "gpt-4o-mini"
    )
    api_key = (
        os.getenv("OPENAI_API_KEY")
        or os.getenv("MEMORY_SERVER__LLM__API_KEY")
        or nested_get(data, "llm.api_key")
        or ""
    )

    return OpenAIConfig(
        endpoint=str(endpoint).rstrip("/"),
        api_key=str(api_key),
        model=str(model),
    )


def request_json(
    method: str,
    url: str,
    *,
    headers: dict[str, str] | None = None,
    payload: dict[str, Any] | None = None,
    timeout: int = 120,
) -> dict[str, Any]:
    try:
        response = requests.request(
            method,
            url,
            headers=headers,
            json=payload,
            timeout=timeout,
        )
    except requests.RequestException as exc:
        raise RuntimeError(f"Request failed: {method} {url}: {exc}") from exc

    if response.status_code >= 400:
        raise RuntimeError(
            f"{method} {url} -> {response.status_code}\n{response.text[:2000]}"
        )

    if not response.text:
        return {}

    try:
        return response.json()
    except ValueError as exc:
        raise RuntimeError(f"{method} {url} did not return JSON: {response.text}") from exc


def ensure_server_ready(base_url: str, timeout: int) -> None:
    health = request_json("GET", f"{base_url}/health", timeout=timeout)
    logger.info(
        "memory server health: status={} postgres={} qdrant={} embedding={}",
        health.get("status", "unknown"),
        nested_get(health, "postgres.status", "unknown"),
        nested_get(health, "qdrant.status", "unknown"),
        nested_get(health, "embedding.status", "unknown"),
    )


def create_profile(base_url: str, timeout: int) -> str:
    suffix = uuid.uuid4().hex[:8]
    payload = {
        "name": f"openai-interactive-memory-{suffix}",
        "description": """
这是一个交互式个人记忆测试系统。
用户会连续输入自然语言片段，系统需要从中抽取可复用的用户记忆。
重点保留长期偏好、预算变化、明确反偏好、职业倾向、生活习惯和稳定事实。
当新输入修正旧信息时，应更新旧记忆，而不是让过时事实继续误导问答。
检索时需要帮助业务侧回答关于用户偏好、约束和推荐注意事项的问题。
""",
    }
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems",
        payload=payload,
        timeout=timeout,
    )
    profile_id = response["id"]
    logger.success("created system profile: {}", profile_id)
    return profile_id


def format_count(value: Any) -> int:
    return int(value) if isinstance(value, int) else 0


def compact(text: Any, limit: int = 180) -> str:
    one_line = " ".join(str(text or "").split())
    if len(one_line) <= limit:
        return one_line
    return one_line[: limit - 3] + "..."


def format_score(value: Any) -> str:
    if isinstance(value, (int, float)):
        return f"{value:.4f}"
    return "n/a"


def log_event_memory_changes(response: dict[str, Any]) -> None:
    event_id = response.get("event_id")
    status = response.get("processing_status")
    skipped = bool(response.get("skipped", False))
    relevance_score = response.get("relevance_score")

    logger.info(
        "event processed: id={} status={} skipped={} relevance={}",
        event_id,
        status,
        skipped,
        format_score(relevance_score),
    )

    summary = response.get("event_summary")
    if summary:
        logger.info("event summary: {}", compact(summary, 240))

    if skipped:
        logger.warning("event skipped: {}", response.get("skip_reason") or "no reason")
        return

    created = format_count(response.get("memories_created"))
    reinforced = format_count(response.get("memories_reinforced"))
    superseded = format_count(response.get("memories_superseded"))

    if created:
        logger.success(
            "memory inserted: count={} ids={}",
            created,
            response.get("created_memory_ids") or [],
        )
    if reinforced:
        logger.success(
            "memory reinforced: count={} ids={}",
            reinforced,
            response.get("reinforced_memory_ids") or [],
        )
    if superseded:
        logger.warning(
            "memory updated/superseded: count={} old_ids={}",
            superseded,
            response.get("superseded_memory_ids") or [],
        )
    if not created and not reinforced and not superseded:
        logger.info("no memory mutation was reported by the server")


def log_related_memories(
    base_url: str,
    identity: SessionIdentity,
    event_id: str | None,
    timeout: int,
) -> None:
    if not event_id:
        return

    try:
        event = request_json(
            "GET",
            f"{base_url}/api/v1/systems/{identity.profile_id}/events/{event_id}",
            timeout=timeout,
        )
    except Exception as exc:
        logger.warning("failed to load related memories for event {}: {}", event_id, exc)
        return

    related = event.get("related_memories") or []
    if not related:
        logger.info("event has no related memories")
        return

    for item in related:
        logger.info(
            "related memory: relation={} id={} preview={}",
            item.get("relation_type"),
            item.get("memory_id"),
            compact(item.get("content_preview"), 220),
        )


def ingest_user_text(
    base_url: str,
    identity: SessionIdentity,
    text: str,
    source: str,
    timeout: int,
) -> None:
    payload = {
        "owner_id": identity.owner_id,
        "scope_id": identity.scope_id,
        "content": text,
        "context": "交互式终端测试输入。请提取对未来问答有帮助的用户记忆。",
        "source": source,
    }
    started = time.time()
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{identity.profile_id}/events",
        payload=payload,
        timeout=timeout,
    )
    elapsed = time.time() - started
    log_event_memory_changes(response)
    log_related_memories(base_url, identity, response.get("event_id"), timeout)
    logger.info("ingest latency: {:.1f}s", elapsed)


def retrieve_memories(
    base_url: str,
    identity: SessionIdentity,
    question: str,
    top_k: int,
    timeout: int,
) -> list[dict[str, Any]]:
    payload = {
        "query": question,
        "owner_id": identity.owner_id,
        "scope_id": identity.scope_id,
        "top_k": top_k,
        "options": {
            "include_evidence": True,
            "include_history": True,
            "use_vector": True,
            "use_fulltext": True,
            "highlight": True,
        },
    }
    started = time.time()
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{identity.profile_id}/memories/retrieve",
        payload=payload,
        timeout=timeout,
    )
    elapsed = time.time() - started
    memories = response.get("memories") or []
    logger.info(
        "retrieval done: candidates={} returned={} latency={:.1f}s",
        response.get("total_candidates"),
        len(memories),
        elapsed,
    )

    for index, item in enumerate(memories, start=1):
        memory = item.get("memory", {})
        logger.info(
            "hit #{} score={} sim={} text={} id={} category={} version={} current={} reinforced={} content={}",
            index,
            format_score(item.get("score")),
            format_score(item.get("similarity")),
            format_score(item.get("text_match_score")),
            memory.get("id"),
            memory.get("category"),
            memory.get("version_number"),
            memory.get("is_current_version"),
            memory.get("reinforcement_count"),
            compact(memory.get("content"), 260),
        )

    return memories


def build_memory_context(memories: list[dict[str, Any]]) -> str:
    lines: list[str] = []
    for index, item in enumerate(memories, start=1):
        memory = item.get("memory", {})
        evidence = item.get("evidence") or {}
        history = item.get("history") or {}
        line = (
            f"[{index}] score={item.get('score')} category={memory.get('category')} "
            f"tags={memory.get('tags') or []} version={memory.get('version_number')} "
            f"current={memory.get('is_current_version')} evidence_count={evidence.get('total_count')} "
            f"history_versions={len(history.get('versions') or [])}\n"
            f"{str(memory.get('content') or '').strip()}"
        )
        lines.append(line)
    return "\n\n".join(lines)


def ask_openai(
    openai_config: OpenAIConfig,
    identity: SessionIdentity,
    question: str,
    memories: list[dict[str, Any]],
    timeout: int,
) -> str:
    if not openai_config.api_key:
        raise RuntimeError(
            "OpenAI API key is empty. Put it in memory-server/config/config.yaml.example "
            "under llm.api_key, or set OPENAI_API_KEY / MEMORY_SERVER__LLM__API_KEY."
        )

    memory_context = build_memory_context(memories)
    payload = {
        "model": openai_config.model,
        "temperature": 0,
        "messages": [
            {
                "role": "system",
                "content": (
                    "你是一个个人记忆问答助手。只能依据提供的检索记忆回答。"
                    "如果记忆不足或冲突，要明确说明不足或冲突，不要编造。"
                    "回答要中文、简洁、可执行，并优先使用当前版本的记忆。"
                ),
            },
            {
                "role": "user",
                "content": (
                    f"profile_id: {identity.profile_id}\n"
                    f"owner_id: {identity.owner_id}\n"
                    f"scope_id: {identity.scope_id}\n\n"
                    f"用户问题：{question}\n\n"
                    f"检索到的记忆：\n{memory_context or '(无)'}\n\n"
                    "请回答用户问题，并列出你依据的关键记忆点。"
                ),
            },
        ],
    }

    started = time.time()
    response = request_json(
        "POST",
        f"{openai_config.endpoint}/chat/completions",
        headers={
            "Authorization": f"Bearer {openai_config.api_key}",
            "Content-Type": "application/json",
        },
        payload=payload,
        timeout=timeout,
    )
    elapsed = time.time() - started
    answer = response["choices"][0]["message"]["content"].strip()
    logger.info("openai answer latency: {:.1f}s model={}", elapsed, openai_config.model)
    return answer


def parse_question(text: str) -> str | None:
    stripped = text.strip()
    if not stripped.startswith("提问"):
        return None
    question = stripped[len("提问") :].lstrip(" ：:，,")
    return question or None


def log_help() -> None:
    logger.info("输入普通文本：写入事件，触发记忆抽取/强化/更新")
    logger.info("输入 `提问 你的问题`：检索记忆，并让 OpenAI 基于记忆回答")
    logger.info("输入 `/ids`：查看当前 profile_id / owner_id / scope_id")
    logger.info("输入 `/help`：查看帮助")
    logger.info("输入 `/exit`、`退出` 或 Ctrl-D：结束会话")


def interactive_loop(
    base_url: str,
    identity: SessionIdentity,
    openai_config: OpenAIConfig,
    source: str,
    top_k: int,
    timeout: int,
) -> None:
    logger.success("interactive memory chat is ready")
    logger.info(
        "profile_id={} owner_id={} scope_id={}",
        identity.profile_id,
        identity.owner_id,
        identity.scope_id,
    )
    log_help()

    while True:
        try:
            text = input("\n你> ").strip()
        except EOFError:
            logger.info("received EOF, bye")
            return
        except KeyboardInterrupt:
            logger.info("interrupted, bye")
            return

        if not text:
            continue

        if text in {"/exit", "exit", "quit", "退出", "结束"}:
            logger.info("bye")
            return
        if text in {"/help", "help", "帮助"}:
            log_help()
            continue
        if text == "/ids":
            logger.info(
                "profile_id={} owner_id={} scope_id={}",
                identity.profile_id,
                identity.owner_id,
                identity.scope_id,
            )
            continue

        question = parse_question(text)
        try:
            if question is None:
                ingest_user_text(base_url, identity, text, source, timeout)
                continue

            logger.info("question mode: {}", question)
            memories = retrieve_memories(base_url, identity, question, top_k, timeout)
            answer = ask_openai(openai_config, identity, question, memories, timeout)
            logger.success("OpenAI answer:\n{}", answer)
        except Exception as exc:
            logger.exception("operation failed: {}", exc)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Interactive terminal chat for memory ingestion and OpenAI QA."
    )
    parser.add_argument(
        "--base-url",
        default="http://localhost:18081",
        help="Memory server base URL.",
    )
    parser.add_argument(
        "--config",
        type=Path,
        default=DEFAULT_CONFIG,
        help="YAML config path used to read OpenAI endpoint/model/api_key.",
    )
    parser.add_argument(
        "--profile-id",
        help="Reuse an existing system profile. If omitted, a new profile is created.",
    )
    parser.add_argument(
        "--owner-id",
        default=f"interactive-user-{uuid.uuid4().hex[:8]}",
        help="Owner ID for this interactive session.",
    )
    parser.add_argument(
        "--scope-id",
        default=f"interactive-scope-{uuid.uuid4().hex[:8]}",
        help="Scope ID for this interactive session.",
    )
    parser.add_argument(
        "--source",
        default="conversation",
        choices=["conversation", "user_action", "system_event", "manual", "api"],
        help="Event source sent to the memory server.",
    )
    parser.add_argument(
        "--top-k",
        type=int,
        default=8,
        help="Number of memories to retrieve per question.",
    )
    parser.add_argument(
        "--timeout",
        type=int,
        default=180,
        help="HTTP timeout in seconds for memory server and OpenAI calls.",
    )
    parser.add_argument(
        "--skip-health",
        action="store_true",
        help="Skip the initial /health request.",
    )
    return parser.parse_args()


def main() -> int:
    setup_logger()
    args = parse_args()
    base_url = "localhost:8080"
    config_path = "../memory-server/config/config.yaml"

    logger.info("reading OpenAI config from: {}", config_path)
    openai_config = load_openai_config(config_path)
    logger.info(
        "OpenAI endpoint={} model={} api_key={}",
        openai_config.endpoint,
        openai_config.model,
        "set" if openai_config.api_key else "empty",
    )

    if not args.skip_health:
        ensure_server_ready(base_url, args.timeout)

    profile_id = args.profile_id or create_profile(base_url, args.timeout)
    identity = SessionIdentity(
        profile_id=profile_id,
        owner_id=args.owner_id,
        scope_id=args.scope_id,
    )
    interactive_loop(
        base_url=base_url,
        identity=identity,
        openai_config=openai_config,
        source=args.source,
        top_k=args.top_k,
        timeout=args.timeout,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
