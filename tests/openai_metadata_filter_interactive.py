#!/usr/bin/env python3
"""
Profile-schema metadata filter interactive test.

Flow:
1. Create or reuse a system profile.
2. Save and confirm the generated metadata_schema.
3. Ingest conversation lines as events.
4. For lines starting with "提问", ask OpenAI to build a metadata_filter from
   the question and profile schema, retrieve memories with that filter, then
   answer from retrieved memories.

Run:
    python3 tests/openai_metadata_filter_interactive.py

Dependencies:
    pip install requests loguru pyyaml
"""

from __future__ import annotations

import argparse
import json
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
DEFAULT_CONFIG = REPO_ROOT / "memory-server" / "config" / "config.yaml"
DEFAULT_SCHEMA_DIR = REPO_ROOT / "tests" / ".metadata_schemas"


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
        backtrace=False,
        diagnose=False,
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
    endpoint = nested_get(data, "llm.endpoint")
    model = nested_get(data, "llm.model")
    api_key = nested_get(data, "llm.api_key")

    missing = [
        field
        for field, value in {
            "llm.endpoint": endpoint,
            "llm.model": model,
            "llm.api_key": api_key,
        }.items()
        if value in (None, "")
    ]
    if missing:
        raise RuntimeError(
            f"Missing required OpenAI config in {config_path}: {', '.join(missing)}"
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


def openai_chat_json(
    openai_config: OpenAIConfig,
    messages: list[dict[str, str]],
    timeout: int,
) -> dict[str, Any]:
    if not openai_config.api_key:
        raise RuntimeError("OpenAI API key is empty in YAML config.")

    payload = {
        "model": openai_config.model,
        "temperature": 0,
        "messages": messages,
        "response_format": {"type": "json_object"},
        "enable_thinking": False,
    }
    started = time.time()
    try:
        response = post_openai_chat(openai_config, payload, timeout)
    except RuntimeError as exc:
        logger.warning("OpenAI JSON mode failed, retry without response_format: {}", exc)
        payload.pop("response_format", None)
        response = post_openai_chat(openai_config, payload, timeout)
    elapsed = time.time() - started
    content = response["choices"][0]["message"]["content"].strip()
    logger.info("OpenAI JSON call latency: {:.1f}s model={}", elapsed, openai_config.model)

    try:
        return json.loads(extract_json_text(content))
    except json.JSONDecodeError as exc:
        raise RuntimeError(f"OpenAI did not return valid JSON: {content}") from exc


def openai_chat_text(
    openai_config: OpenAIConfig,
    messages: list[dict[str, str]],
    timeout: int,
) -> str:
    if not openai_config.api_key:
        raise RuntimeError("OpenAI API key is empty in YAML config.")

    payload = {
        "model": openai_config.model,
        "temperature": 0,
        "messages": messages,
        "enable_thinking": False,
    }
    started = time.time()
    response = post_openai_chat(openai_config, payload, timeout)
    elapsed = time.time() - started
    logger.info("OpenAI answer latency: {:.1f}s model={}", elapsed, openai_config.model)
    return response["choices"][0]["message"]["content"].strip()


def post_openai_chat(
    openai_config: OpenAIConfig,
    payload: dict[str, Any],
    timeout: int,
) -> dict[str, Any]:
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
    return response


def extract_json_text(text: str) -> str:
    stripped = text.strip()
    if stripped.startswith("```"):
        lines = stripped.splitlines()
        if lines and lines[0].startswith("```"):
            lines = lines[1:]
        if lines and lines[-1].strip() == "```":
            lines = lines[:-1]
        stripped = "\n".join(lines).strip()

    start = stripped.find("{")
    end = stripped.rfind("}")
    if start >= 0 and end >= start:
        return stripped[start : end + 1]
    return stripped


def ensure_server_ready(base_url: str, timeout: int) -> None:
    health = request_json("GET", f"{base_url}/health", timeout=timeout)
    logger.info(
        "memory server health: status={} postgres={} qdrant={} embedding={}",
        health.get("status", "unknown"),
        nested_get(health, "postgres.status", "unknown"),
        nested_get(health, "qdrant.status", "unknown"),
        nested_get(health, "embedding.status", "unknown"),
    )


def compact(text: Any, limit: int = 180) -> str:
    one_line = " ".join(str(text or "").split())
    if len(one_line) <= limit:
        return one_line
    return one_line[: limit - 3] + "..."


def pretty_json(value: Any) -> str:
    return json.dumps(value, ensure_ascii=False, indent=2, sort_keys=True)


def format_score(value: Any) -> str:
    if isinstance(value, (int, float)):
        return f"{value:.4f}"
    return "n/a"


def prompt_multiline(prompt: str, default: str | None = None) -> str:
    logger.info(prompt)
    logger.info("多行输入，单独一行输入 `.` 结束")
    lines: list[str] = []
    while True:
        line = input("> ")
        if line.strip() == ".":
            break
        lines.append(line)
    text = "\n".join(lines).strip()
    if text:
        return text
    if default is not None:
        return default
    raise RuntimeError("input cannot be empty")


def default_profile_description() -> str:
    return """
这是一个个人长期记忆测试系统。
系统服务于一个会长期对话的 AI 助手，目标是保存、更新和检索用户的稳定事实、偏好、反偏好、技能、工作方式、生活习惯、目标变化和历史状态。
metadata schema 应该帮助业务侧按主题、对象、时间属性、偏好方向、领域、变化类型等字段做检索缩圈。
当用户表达“以前/去年/过去”和“现在/今年/以后”的变化时，应保留变化链路，并能检索到当前版本。
边界：不要保存一次性闲聊、无长期价值的情绪噪声、敏感隐私推断。
""".strip()


def create_profile_from_input(base_url: str, timeout: int) -> dict[str, Any]:
    suffix = uuid.uuid4().hex[:8]
    name = input(f"system profile name [metadata-filter-test-{suffix}]> ").strip()
    if not name:
        name = f"metadata-filter-test-{suffix}"

    description = prompt_multiline(
        "请输入 system profile 描述；直接输入 `.` 使用默认个人记忆 profile。",
        default_profile_description(),
    )

    payload = {"name": name, "description": description}
    started = time.time()
    profile = request_json(
        "POST",
        f"{base_url}/api/v1/systems",
        payload=payload,
        timeout=timeout,
    )
    logger.success(
        "created system profile: id={} schema_status={} schema_version={} latency={:.1f}s",
        profile.get("id"),
        profile.get("schema_status"),
        profile.get("schema_version"),
        time.time() - started,
    )
    return profile


def load_profile(base_url: str, profile_id: str, timeout: int) -> dict[str, Any]:
    profile = request_json(
        "GET",
        f"{base_url}/api/v1/systems/{profile_id}",
        timeout=timeout,
    )
    logger.success(
        "loaded system profile: id={} schema_status={} schema_version={}",
        profile.get("id"),
        profile.get("schema_status"),
        profile.get("schema_version"),
    )
    return profile


def confirm_schema(base_url: str, profile_id: str, timeout: int) -> dict[str, Any]:
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/schema/confirm",
        timeout=timeout,
    )
    profile = response.get("profile") or {}
    logger.success(
        "confirmed metadata schema: status={} version={} confirmed_at={}",
        profile.get("schema_status"),
        profile.get("schema_version"),
        profile.get("schema_confirmed_at"),
    )
    return profile


def save_schema(profile: dict[str, Any], schema_dir: Path) -> Path:
    schema_dir.mkdir(parents=True, exist_ok=True)
    profile_id = str(profile["id"])
    path = schema_dir / f"{profile_id}.metadata_schema.json"
    payload = {
        "profile_id": profile_id,
        "name": profile.get("name"),
        "purpose": profile.get("purpose"),
        "domain": profile.get("domain"),
        "schema_status": profile.get("schema_status"),
        "schema_version": profile.get("schema_version"),
        "metadata_schema": profile.get("metadata_schema") or {},
        "schema_generation_prompt": profile.get("schema_generation_prompt"),
    }
    path.write_text(pretty_json(payload) + "\n", encoding="utf-8")
    logger.success("saved metadata schema: {}", path)
    return path


def log_profile_schema(profile: dict[str, Any]) -> None:
    logger.info(
        "profile summary: name={} purpose={} domain={} target={}",
        profile.get("name"),
        compact(profile.get("purpose"), 120),
        profile.get("domain"),
        compact(profile.get("target_audience"), 120),
    )
    schema = profile.get("metadata_schema") or {}
    logger.info("filterable_fields: {}", schema.get("filterable_fields") or [])
    logger.info("metadata_schema:\n{}", pretty_json(schema))
    prompt = profile.get("schema_generation_prompt")
    if prompt:
        logger.info("schema_generation_prompt:\n{}", compact(prompt, 800))


def format_count(value: Any) -> int:
    return int(value) if isinstance(value, int) else 0


def log_event_memory_changes(response: dict[str, Any]) -> None:
    logger.info(
        "event processed: id={} status={} skipped={} relevance={}",
        response.get("event_id"),
        response.get("processing_status"),
        bool(response.get("skipped", False)),
        format_score(response.get("relevance_score")),
    )

    summary = response.get("event_summary")
    if summary:
        logger.info("event summary: {}", compact(summary, 260))

    payload = response.get("analysis_payload")
    if payload:
        logger.info("analysis_payload:\n{}", pretty_json(payload))

    if response.get("skipped"):
        logger.warning("event skipped: {}", response.get("skip_reason") or "no reason")
        return

    created = format_count(response.get("memories_created"))
    reinforced = format_count(response.get("memories_reinforced"))
    superseded = format_count(response.get("memories_superseded"))

    if created:
        logger.success("memory inserted: count={} ids={}", created, response.get("created_memory_ids") or [])
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


def log_related_memories(base_url: str, identity: SessionIdentity, event_id: str | None, timeout: int) -> None:
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

    for item in event.get("related_memories") or []:
        logger.info(
            "related memory: relation={} id={} preview={}",
            item.get("relation_type"),
            item.get("memory_id"),
            compact(item.get("content_preview"), 260),
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
        "context": "metadata schema/filter 交互式测试输入。请抽取符合当前 profile metadata_schema 的长期记忆。",
        "source": source,
    }
    logger.info("ingest request:\n{}", pretty_json(payload))
    started = time.time()
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{identity.profile_id}/events",
        payload=payload,
        timeout=timeout,
    )
    log_event_memory_changes(response)
    log_related_memories(base_url, identity, response.get("event_id"), timeout)
    logger.info("ingest latency: {:.1f}s", time.time() - started)


def empty_filter(reason: str = "no reliable metadata condition") -> dict[str, Any]:
    return {
        "metadata_filter": {"where": []},
        "reasoning": reason,
        "expected_recall": "Use semantic/fulltext retrieval without metadata narrowing.",
    }


def build_metadata_filter_with_openai(
    openai_config: OpenAIConfig,
    profile: dict[str, Any],
    question: str,
    timeout: int,
) -> dict[str, Any]:
    schema = profile.get("metadata_schema") or {}
    schema_generation_prompt = profile.get("schema_generation_prompt") or ""
    filterable_fields = schema.get("filterable_fields") or []
    if not filterable_fields:
        logger.warning("profile schema has no filterable_fields; skip filter planning")
        return empty_filter("profile metadata_schema.filterable_fields is empty")

    messages = [
        {
            "role": "system",
            "content": (
                "你是 memory retrieval metadata filter planner。"
                "你只能输出 JSON object，不要输出 markdown。"
                "目标是基于用户问题和 metadata_schema 构造保守的 metadata_filter。"
                "metadata_filter 结构固定为 {\"where\": [{\"field\": {\"eq\": value}}]}。"
                "where 中多个 clause 以及同一 clause 多字段都是 AND。"
                "支持操作符：eq, ne, in, exists。"
                "只能使用 metadata_schema.filterable_fields 中声明的字段。"
                "如果问题不能可靠映射到 schema 字段，返回空 filter：{\"where\": []}。"
                "不要为了显得聪明而杜撰 schema 中没有的字段或枚举值。"
                "如果 schema_generation_prompt 或 metadata_schema 给了字段允许值，你必须使用其中的 canonical value，而不是用户问题里的自然语言别名。"
                "例如用户说“购房能力”，如果 schema 里只有 budget，就应该映射为 budget；如果无法可靠映射，则返回空 filter。"
            ),
        },
        {
            "role": "user",
            "content": (
                f"profile purpose: {profile.get('purpose')}\n"
                f"profile domain: {profile.get('domain')}\n"
                f"filterable_fields: {pretty_json(filterable_fields)}\n"
                f"metadata_schema:\n{pretty_json(schema)}\n\n"
                f"schema_generation_prompt:\n{schema_generation_prompt}\n\n"
                f"用户问题：{question}\n\n"
                "请返回：\n"
                "{\n"
                "  \"metadata_filter\": {\"where\": [...]},\n"
                "  \"reasoning\": \"为什么这样过滤，或为什么不过滤\",\n"
                "  \"expected_recall\": \"希望缩圈到哪类记忆\"\n"
                "}"
            ),
        },
    ]

    try:
        plan = openai_chat_json(openai_config, messages, timeout)
    except Exception as exc:
        logger.warning("filter planning failed, fallback to empty filter: {}", exc)
        return empty_filter(str(exc))

    metadata_filter = plan.get("metadata_filter")
    if not isinstance(metadata_filter, dict):
        plan["metadata_filter"] = {"where": []}
    if not isinstance(plan["metadata_filter"].get("where"), list):
        plan["metadata_filter"]["where"] = []
    return plan


def retrieve_memories(
    base_url: str,
    identity: SessionIdentity,
    question: str,
    metadata_filter: dict[str, Any],
    top_k: int,
    timeout: int,
) -> list[dict[str, Any]]:
    payload = {
        "query": question,
        "owner_id": identity.owner_id,
        "scope_id": identity.scope_id,
        "top_k": top_k,
        "metadata_filter": metadata_filter,
        "options": {
            "include_evidence": True,
            "include_history": True,
            "use_vector": True,
            "use_fulltext": True,
            "highlight": True,
        },
    }
    logger.info("retrieval request:\n{}", pretty_json(payload))
    started = time.time()
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{identity.profile_id}/memories/retrieve",
        payload=payload,
        timeout=timeout,
    )
    elapsed = time.time() - started
    memories = response.get("memories") or []
    trace = response.get("trace") or {}
    logger.info(
        "retrieval done: candidates={} metadata_filtered={} returned={} filter_applied={} latency={:.1f}s",
        response.get("total_candidates"),
        trace.get("metadata_filtered_candidate_count"),
        len(memories),
        trace.get("metadata_filter_applied"),
        elapsed,
    )
    logger.info("retrieval trace:\n{}", pretty_json(trace))

    for index, item in enumerate(memories, start=1):
        memory = item.get("memory", {})
        logger.info(
            "hit #{} score={} sim={} text={} id={} version={} current={} reinforced={} content={}",
            index,
            format_score(item.get("score")),
            format_score(item.get("similarity")),
            format_score(item.get("text_match_score")),
            memory.get("id"),
            memory.get("version_number"),
            memory.get("is_current_version"),
            memory.get("reinforcement_count"),
            compact(memory.get("content"), 280),
        )
        logger.info("hit #{} metadata:\n{}", index, pretty_json(memory.get("metadata") or {}))
        lineage = item.get("merged_lineage_sources")
        if lineage:
            logger.info("hit #{} lineage_sources:\n{}", index, pretty_json(lineage))

    return memories


def build_memory_context(memories: list[dict[str, Any]]) -> str:
    lines: list[str] = []
    for index, item in enumerate(memories, start=1):
        memory = item.get("memory", {})
        evidence = item.get("evidence") or {}
        history = item.get("history") or {}
        line = (
            f"[{index}] score={item.get('score')} id={memory.get('id')} "
            f"version={memory.get('version_number')} current={memory.get('is_current_version')} "
            f"evidence_count={evidence.get('total_count')} "
            f"history_versions={len(history.get('versions') or [])}\n"
            f"metadata={json.dumps(memory.get('metadata') or {}, ensure_ascii=False)}\n"
            f"{str(memory.get('content') or '').strip()}"
        )
        lines.append(line)
    return "\n\n".join(lines)


def answer_with_openai(
    openai_config: OpenAIConfig,
    identity: SessionIdentity,
    question: str,
    filter_plan: dict[str, Any],
    memories: list[dict[str, Any]],
    timeout: int,
) -> str:
    memory_context = build_memory_context(memories)
    messages = [
        {
            "role": "system",
            "content": (
                "你是个人记忆问答助手。只能依据检索到的记忆回答。"
                "如果记忆不足或检索条件可能过窄，要明确说明。"
                "优先使用 current=true 的当前版本记忆。中文回答，简洁但要列出依据。"
            ),
        },
        {
            "role": "user",
            "content": (
                f"profile_id: {identity.profile_id}\n"
                f"owner_id: {identity.owner_id}\n"
                f"scope_id: {identity.scope_id}\n\n"
                f"用户问题：{question}\n\n"
                f"metadata filter plan:\n{pretty_json(filter_plan)}\n\n"
                f"检索到的记忆：\n{memory_context or '(无)'}\n\n"
                "请回答用户问题，并列出你依据的关键记忆点。"
            ),
        },
    ]
    return openai_chat_text(openai_config, messages, timeout)


def parse_question(text: str) -> str | None:
    stripped = text.strip()
    if not stripped.startswith("提问"):
        return None
    question = stripped[len("提问") :].lstrip(" ：:，,")
    return question or None


def log_help() -> None:
    logger.info("输入普通文本：写入事件，触发 profile-aware metadata 记忆抽取")
    logger.info("输入 `提问 你的问题`：OpenAI 先规划 metadata_filter，再检索并回答")
    logger.info("输入 `/schema`：打印当前 metadata_schema")
    logger.info("输入 `/ids`：查看当前 profile_id / owner_id / scope_id")
    logger.info("输入 `/help`：查看帮助")
    logger.info("输入 `/exit`、`退出` 或 Ctrl-D：结束会话")


def interactive_loop(
    base_url: str,
    identity: SessionIdentity,
    profile: dict[str, Any],
    openai_config: OpenAIConfig,
    source: str,
    top_k: int,
    timeout: int,
) -> None:
    logger.success("metadata filter interactive test is ready")
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
        if text == "/schema":
            log_profile_schema(profile)
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
            filter_plan = build_metadata_filter_with_openai(
                openai_config,
                profile,
                question,
                timeout,
            )
            logger.info("metadata filter plan:\n{}", pretty_json(filter_plan))
            metadata_filter = filter_plan.get("metadata_filter") or {"where": []}
            memories = retrieve_memories(
                base_url,
                identity,
                question,
                metadata_filter,
                top_k,
                timeout,
            )
            answer = answer_with_openai(
                openai_config,
                identity,
                question,
                filter_plan,
                memories,
                timeout,
            )
            logger.success("OpenAI answer:\n{}", answer)
        except Exception as exc:
            logger.error("operation failed: {}", exc)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Interactive metadata-schema filter test with OpenAI planning."
    )
    parser.add_argument("--base-url", default="http://localhost:8080")
    parser.add_argument("--config", type=Path, default=DEFAULT_CONFIG)
    parser.add_argument("--profile-id", help="Reuse an existing system profile.")
    parser.add_argument(
        "--owner-id",
        default=f"metadata-filter-user-{uuid.uuid4().hex[:8]}",
    )
    parser.add_argument(
        "--scope-id",
        default=f"metadata-filter-scope-{uuid.uuid4().hex[:8]}",
    )
    parser.add_argument(
        "--source",
        default="conversation",
        choices=["conversation", "user_action", "system_event", "manual", "api"],
    )
    parser.add_argument("--top-k", type=int, default=8)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--skip-health", action="store_true")
    parser.add_argument("--skip-confirm", action="store_true")
    parser.add_argument("--schema-dir", type=Path, default=DEFAULT_SCHEMA_DIR)
    return parser.parse_args()


def main() -> int:
    setup_logger()
    args = parse_args()

    logger.info("reading OpenAI config from: {}", args.config)
    openai_config = load_openai_config(args.config)
    logger.info(
        "OpenAI endpoint={} model={} api_key={}",
        openai_config.endpoint,
        openai_config.model,
        "set" if openai_config.api_key else "empty",
    )

    if not args.skip_health:
        ensure_server_ready(args.base_url, args.timeout)

    profile = (
        load_profile(args.base_url, args.profile_id, args.timeout)
        if args.profile_id
        else create_profile_from_input(args.base_url, args.timeout)
    )
    log_profile_schema(profile)
    save_schema(profile, args.schema_dir)

    if not args.skip_confirm and profile.get("schema_status") != "confirmed":
        answer = input("confirm this metadata_schema? [Y/n]> ").strip().lower()
        if answer in {"", "y", "yes"}:
            profile = confirm_schema(args.base_url, str(profile["id"]), args.timeout)
            save_schema(profile, args.schema_dir)
        else:
            logger.warning("schema not confirmed; continuing test with draft schema")

    identity = SessionIdentity(
        profile_id=str(profile["id"]),
        owner_id=args.owner_id,
        scope_id=args.scope_id,
    )
    interactive_loop(
        base_url=args.base_url,
        identity=identity,
        profile=profile,
        openai_config=openai_config,
        source=args.source,
        top_k=args.top_k,
        timeout=args.timeout,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
