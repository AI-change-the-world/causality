#!/usr/bin/env python3
"""
OpenAI-backed question-answering test for the memory server.

Flow:
1. Create a system profile.
2. Ingest several user events through the memory server.
3. Retrieve memories for each question.
4. Ask OpenAI to answer using only retrieved memories.
5. Run lightweight keyword checks over retrieval + answer text.

Run:
    python3 tests/openai_qa_test.py --base-url http://localhost:18081

By default this reads OpenAI defaults from:
    memory-server/config/config.yaml.example

Environment variables can override the OpenAI call used by this script:
    OPENAI_API_KEY
    OPENAI_BASE_URL
    OPENAI_MODEL
    MEMORY_SERVER__LLM__API_KEY
    MEMORY_SERVER__LLM__ENDPOINT
    MEMORY_SERVER__LLM__MODEL
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


REPO_ROOT = Path(__file__).resolve().parents[1]
DEFAULT_CONFIG = REPO_ROOT / "memory-server" / "config" / "config.yaml.example"


@dataclass
class OpenAIConfig:
    endpoint: str
    api_key: str
    model: str


@dataclass
class QuestionCase:
    name: str
    question: str
    expected_groups: list[list[str]]


EVENTS = [
    {
        "content": """
用户在看房对话里说：我一直想买一套海景房，最好能看到海。
居住环境必须安静，小区不要被游客围着，也不想住在夜生活很吵的旅游区。
当前预算大概只有 300 万，所以太贵的房子暂时买不起。
如果能换来安静和海景，离地铁稍微远一点也可以接受。
""",
        "context": "用户和私人助理讨论购房偏好，需要提取长期可复用的居住偏好。",
        "source": "conversation",
    },
    {
        "content": """
几天后用户补充：最近中彩票，资金突然充裕了。
买房预算可以提高到 800 万左右，之前预算只有 300 万已经不准确了。
海景和安静依然最重要，但我还是不喜欢游客很多、夜里很吵的区域。
""",
        "context": "这是对购房预算的更新，同时保留原有位置和环境偏好。",
        "source": "conversation",
    },
    {
        "content": """
用户整理代码仓库时说：我越来越不想长期做纯 CRUD 项目。
不是不能写接口、表和字段，而是这种重复工作很消耗耐心。
如果可以选，我更愿意做偏架构、偏工具链、偏抽象和系统可靠性的事情。
我喜欢性能够用但清晰安全的方案，也讨厌只打补丁、没有结构改进的工作。
""",
        "context": "用户在描述技术工作偏好，需要记录职业/项目方向偏好。",
        "source": "conversation",
    },
    {
        "content": """
用户后来又说：我现在能理解 CRUD 在业务早期快速交付时有价值。
但如果是长期职业方向，我仍然不想只做重复 CRUD。
我更想负责工具、架构优化、系统设计和工程质量这些能改善结构的问题。
""",
        "context": "用户修正了对 CRUD 的绝对看法，但长期偏好没有反转。",
        "source": "conversation",
    },
    {
        "content": """
用户随口提到：最近下午咖啡喝多了，晚上有点睡不着。
准备先把下午的咖啡减掉，观察睡眠有没有改善。
""",
        "context": "生活习惯信息，用来测试检索时是否能避免无关记忆干扰。",
        "source": "conversation",
    },
]


QUESTIONS = [
    QuestionCase(
        name="housing",
        question="如果我要帮这个用户筛选房子，应该记住他的预算和居住偏好吗？",
        expected_groups=[
            ["800", "八百万", "800万", "800 万"],
            ["海景", "看海", "海边"],
            ["安静", "安宁"],
            ["旅游区", "游客", "吵", "噪音"],
        ],
    ),
    QuestionCase(
        name="work",
        question="如果给这个用户推荐技术工作内容，哪些方向更适合，哪些要避免？",
        expected_groups=[
            ["架构", "系统设计"],
            ["工具", "工具链"],
            ["CRUD", "重复"],
            ["结构改进", "工程质量", "系统可靠性"],
        ],
    ),
]


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
        body = response.text[:2000]
        raise RuntimeError(f"{method} {url} -> {response.status_code}\n{body}")

    if not response.text:
        return {}
    try:
        return response.json()
    except ValueError as exc:
        raise RuntimeError(f"{method} {url} did not return JSON: {response.text}") from exc


def ensure_server_ready(base_url: str, timeout: int) -> None:
    url = f"{base_url}/health"
    health = request_json("GET", url, timeout=timeout)
    status = health.get("status", "unknown")
    print(f"[health] memory server status: {status}")


def create_profile(base_url: str, timeout: int) -> str:
    suffix = uuid.uuid4().hex[:8]
    payload = {
        "name": f"openai-qa-memory-test-{suffix}",
        "description": """
这是一个个人助手记忆系统，用于从用户的聊天、日记和操作记录中抽取可复用记忆。
系统需要优先保留用户偏好、预算变化、职业方向、长期倾向和明确的反偏好。
系统不应该把纯噪音、临时情绪或无关闲聊当成高价值长期记忆。
检索时应帮助业务侧回答关于用户偏好、决策约束和推荐注意事项的问题。
""",
    }
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems",
        payload=payload,
        timeout=timeout,
    )
    profile_id = response["id"]
    print(f"[profile] created: {profile_id}")
    return profile_id


def ingest_events(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    timeout: int,
) -> None:
    url = f"{base_url}/api/v1/systems/{profile_id}/events"
    for index, event in enumerate(EVENTS, start=1):
        payload = {
            "owner_id": owner_id,
            "scope_id": scope_id,
            "content": event["content"].strip(),
            "context": event["context"],
            "source": event["source"],
        }
        response = request_json("POST", url, payload=payload, timeout=timeout)
        status = response.get("processing_status")
        skipped = response.get("skipped", False)
        created = response.get("memories_created", 0)
        reinforced = response.get("memories_reinforced", 0)
        superseded = response.get("memories_superseded", 0)
        print(
            "[event:{idx}] id={event_id} status={status} skipped={skipped} "
            "created={created} reinforced={reinforced} superseded={superseded}".format(
                idx=index,
                event_id=response.get("event_id"),
                status=status,
                skipped=skipped,
                created=created,
                reinforced=reinforced,
                superseded=superseded,
            )
        )


def retrieve_memories(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    question: str,
    top_k: int,
    timeout: int,
) -> list[dict[str, Any]]:
    payload = {
        "query": question,
        "owner_id": owner_id,
        "scope_id": scope_id,
        "top_k": top_k,
        "options": {
            "include_evidence": True,
            "include_history": True,
            "use_vector": True,
            "use_fulltext": True,
            "highlight": True,
        },
    }
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/memories/retrieve",
        payload=payload,
        timeout=timeout,
    )
    memories = response.get("memories", [])
    print(
        f"[retrieve] question={question!r} candidates={response.get('total_candidates')} "
        f"returned={len(memories)}"
    )
    for index, item in enumerate(memories[:5], start=1):
        memory = item.get("memory", {})
        content = " ".join(str(memory.get("content", "")).split())
        score = item.get("score")
        score_text = f"{score:.4f}" if isinstance(score, (int, float)) else "n/a"
        print(
            f"  [{index}] score={score_text} "
            f"category={memory.get('category')} content={content[:180]}"
        )
    return memories


def build_memory_context(memories: list[dict[str, Any]]) -> str:
    lines: list[str] = []
    for index, item in enumerate(memories, start=1):
        memory = item.get("memory", {})
        content = str(memory.get("content", "")).strip()
        category = memory.get("category")
        score = item.get("score")
        tags = memory.get("tags") or []
        line = f"[{index}] score={score} category={category} tags={tags}\n{content}"
        lines.append(line)
    return "\n\n".join(lines)


def ask_openai(
    openai_config: OpenAIConfig,
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
                    "你是一个记忆问答测试助手。只能依据提供的用户记忆回答，"
                    "如果记忆不足就明确说不足。用中文简洁回答，不要编造。"
                ),
            },
            {
                "role": "user",
                "content": (
                    f"用户问题：{question}\n\n"
                    f"检索到的用户记忆：\n{memory_context}\n\n"
                    "请根据这些记忆回答问题，并指出关键依据。"
                ),
            },
        ],
    }
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
    return response["choices"][0]["message"]["content"].strip()


def missing_expected_groups(text: str, groups: list[list[str]]) -> list[list[str]]:
    normalized = text.lower()
    missing = []
    for group in groups:
        if not any(term.lower() in normalized for term in group):
            missing.append(group)
    return missing


def run_question_case(
    case: QuestionCase,
    *,
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    openai_config: OpenAIConfig,
    top_k: int,
    timeout: int,
    no_openai_answer: bool,
) -> bool:
    print(f"\n[question:{case.name}] {case.question}")
    memories = retrieve_memories(
        base_url,
        profile_id,
        owner_id,
        scope_id,
        case.question,
        top_k,
        timeout,
    )

    answer = ""
    if no_openai_answer:
        print("[answer] skipped by --no-openai-answer")
    else:
        answer = ask_openai(openai_config, case.question, memories, timeout)
        print(f"[answer]\n{answer}")

    combined_text = build_memory_context(memories) + "\n" + answer
    missing = missing_expected_groups(combined_text, case.expected_groups)
    if missing:
        print(f"[check:{case.name}] FAIL missing keyword groups: {missing}")
        return False

    print(f"[check:{case.name}] PASS")
    return True


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(
        description="Run an OpenAI-backed memory retrieval QA test."
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
        help="Reuse an existing system profile instead of creating a new one.",
    )
    parser.add_argument(
        "--owner-id",
        default=f"openai-qa-user-{uuid.uuid4().hex[:8]}",
        help="Owner ID for the test user.",
    )
    parser.add_argument(
        "--scope-id",
        default=f"openai-qa-session-{uuid.uuid4().hex[:8]}",
        help="Scope ID for the test events and retrieval.",
    )
    parser.add_argument(
        "--skip-ingest",
        action="store_true",
        help="Skip event ingestion and only run retrieval + QA.",
    )
    parser.add_argument(
        "--no-openai-answer",
        action="store_true",
        help="Only test retrieval; do not call OpenAI chat completions.",
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
    return parser.parse_args()


def main() -> int:
    args = parse_args()
    base_url = args.base_url.rstrip("/")
    config_path = args.config.resolve()

    print(f"[config] reading OpenAI config from: {config_path}")
    openai_config = load_openai_config(config_path)
    print(
        f"[config] OpenAI endpoint={openai_config.endpoint} "
        f"model={openai_config.model} api_key={'set' if openai_config.api_key else 'empty'}"
    )

    ensure_server_ready(base_url, args.timeout)

    profile_id = args.profile_id or create_profile(base_url, args.timeout)
    print(f"[identity] owner_id={args.owner_id} scope_id={args.scope_id}")

    if args.skip_ingest:
        print("[ingest] skipped by --skip-ingest")
    else:
        started = time.time()
        ingest_events(base_url, profile_id, args.owner_id, args.scope_id, args.timeout)
        print(f"[ingest] done in {time.time() - started:.1f}s")

    results = [
        run_question_case(
            case,
            base_url=base_url,
            profile_id=profile_id,
            owner_id=args.owner_id,
            scope_id=args.scope_id,
            openai_config=openai_config,
            top_k=args.top_k,
            timeout=args.timeout,
            no_openai_answer=args.no_openai_answer,
        )
        for case in QUESTIONS
    ]

    if all(results):
        print("\n[result] PASS: retrieval and QA checks look good.")
        return 0

    print("\n[result] FAIL: at least one QA check did not find expected memory signals.")
    return 1


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except KeyboardInterrupt:
        print("\nInterrupted.", file=sys.stderr)
        raise SystemExit(130)
    except Exception as exc:
        print(f"\nERROR: {exc}", file=sys.stderr)
        raise SystemExit(1)
