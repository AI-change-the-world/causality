#!/usr/bin/env python3
"""
Lineage chain verification script for the memory server.

What it does:
1. Creates a fresh profile.
2. Seeds several direct memories to verify ordinary retrieval behavior.
3. Tries to trigger a supersede chain using closely related event updates.
4. Prints /history, /resolve-current, and /retrieve results with loguru.

Important note:
- The current data model is a single supersede chain:
  one memory can supersede only one previous memory.
- "multiple old memories -> one new memory" is NOT supported by the current schema.

Run:
    python3 tests/lineage_chain_test.py
"""

from __future__ import annotations

import sys
import time
import uuid
from pathlib import Path
from typing import Any

import requests

try:
    from loguru import logger
except ImportError as exc:
    print("Missing dependency: pip install loguru", file=sys.stderr)
    raise SystemExit(1) from exc


BASE_URL = "http://localhost:18081"
REPO_ROOT = Path(__file__).resolve().parents[1]
CONFIG_PATH = REPO_ROOT / "memory-server" / "config" / "config.yaml"


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


def request_json(
    method: str,
    url: str,
    *,
    payload: dict[str, Any] | None = None,
    timeout: int = 180,
) -> dict[str, Any]:
    try:
        response = requests.request(method, url, json=payload, timeout=timeout)
    except requests.RequestException as exc:
        raise RuntimeError(f"Request failed: {method} {url}: {exc}") from exc

    if response.status_code >= 400:
        raise RuntimeError(f"{method} {url} -> {response.status_code}\n{response.text[:2000]}")

    return response.json() if response.text else {}


def compact(value: Any, limit: int = 120) -> str:
    text = " ".join(str(value or "").split())
    if len(text) <= limit:
        return text
    return text[: limit - 3] + "..."


def create_profile(base_url: str, timeout: int) -> str:
    payload = {
        "name": f"lineage-chain-test-{uuid.uuid4().hex[:8]}",
        "description": (
            "用于测试 supersede 链路、resolve-current 以及常规 retrieve 的链路合并行为。"
        ),
    }
    response = request_json("POST", f"{base_url}/api/v1/systems", payload=payload, timeout=timeout)
    return response["id"]


def create_memory(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    *,
    content: str,
    category: str,
    tags: list[str],
    importance: float = 0.8,
    confidence: float = 0.95,
    timeout: int = 180,
) -> str:
    payload = {
        "owner_id": owner_id,
        "scope_id": scope_id,
        "content": content,
        "category": category,
        "tags": tags,
        "importance": importance,
        "confidence": confidence,
        "is_global": False,
    }
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/memories",
        payload=payload,
        timeout=timeout,
    )
    return response["id"]


def create_event(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    *,
    content: str,
    timeout: int = 180,
) -> dict[str, Any]:
    payload = {
        "owner_id": owner_id,
        "scope_id": scope_id,
        "content": content,
        "context": None,
        "source": "conversation",
    }
    return request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/events",
        payload=payload,
        timeout=timeout,
    )


def get_memory(base_url: str, profile_id: str, memory_id: str, timeout: int) -> dict[str, Any]:
    return request_json(
        "GET",
        f"{base_url}/api/v1/systems/{profile_id}/memories/{memory_id}",
        timeout=timeout,
    )


def get_history(base_url: str, profile_id: str, memory_id: str, timeout: int) -> dict[str, Any]:
    return request_json(
        "GET",
        f"{base_url}/api/v1/systems/{profile_id}/memories/{memory_id}/history",
        timeout=timeout,
    )


def resolve_current(
    base_url: str,
    profile_id: str,
    memory_id: str,
    timeout: int,
) -> dict[str, Any]:
    return request_json(
        "GET",
        f"{base_url}/api/v1/systems/{profile_id}/memories/{memory_id}/resolve-current",
        timeout=timeout,
    )


def retrieve(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    *,
    query: str,
    top_k: int = 10,
    include_history: bool = True,
    timeout: int = 180,
) -> dict[str, Any]:
    payload = {
        "query": query,
        "owner_id": owner_id,
        "scope_id": scope_id,
        "top_k": top_k,
        "options": {
            "use_vector": True,
            "use_fulltext": True,
            "include_history": include_history,
            "include_evidence": False,
            "highlight": False,
        },
    }
    return request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/memories/retrieve",
        payload=payload,
        timeout=timeout,
    )


def log_history(history: dict[str, Any]) -> None:
    logger.info(
        "history requested_memory_id={} current_version_id={} hops_to_current={} lineage={}",
        history.get("requested_memory_id"),
        history.get("current_version_id"),
        history.get("hops_to_current"),
        history.get("lineage_to_current"),
    )
    for version in history.get("versions", []):
        logger.info(
            "  version={} current={} id={} supersedes={} superseded_by={} content={}",
            version.get("version_number"),
            version.get("is_current_version"),
            version.get("id"),
            version.get("supersedes"),
            version.get("superseded_by"),
            compact(version.get("content"), 100),
        )


def log_resolve_result(result: dict[str, Any]) -> None:
    current = result.get("current_memory", {})
    logger.info(
        "resolve-current requested={} current={} hops={} lineage={}",
        result.get("requested_memory_id"),
        current.get("id"),
        result.get("hops_to_current"),
        result.get("lineage_to_current"),
    )
    logger.info(
        "  current content={} category={}",
        compact(current.get("content"), 100),
        current.get("category"),
    )


def log_retrieve_result(result: dict[str, Any]) -> None:
    logger.info(
        "retrieve total_candidates={} returned={}",
        result.get("total_candidates"),
        len(result.get("memories", [])),
    )
    for index, item in enumerate(result.get("memories", []), start=1):
        memory = item.get("memory", {})
        logger.info(
            "  hit#{} score={:.4f} id={} version={} current={} content={}",
            index,
            float(item.get("score", 0.0)),
            memory.get("id"),
            memory.get("version_number"),
            memory.get("is_current_version"),
            compact(memory.get("content"), 100),
        )
        lineage_sources = item.get("merged_lineage_sources") or []
        for source in lineage_sources:
            logger.info(
                "    merged source={} path={}",
                source.get("source_memory_id"),
                source.get("lineage_to_current"),
            )


def seed_direct_memories(base_url: str, profile_id: str, owner_id: str, scope_id: str, timeout: int) -> list[str]:
    logger.info("seeding direct memories for ordinary retrieval checks")
    direct_ids = []
    direct_cases = [
        {
            "content": "用户当前更喜欢 Rust 编程语言，欣赏它在系统级编程中的安全性和约束清晰。",
            "category": "preference.tech.programming_language",
            "tags": ["rust", "programming", "systems"],
        },
        {
            "content": "用户去年有较多 C++ 开发经历，但当前不希望把它作为主要偏好语言。",
            "category": "history.tech.cpp",
            "tags": ["cpp", "history"],
        },
        {
            "content": "用户对 Python 作为主要系统工具语言兴趣不高。",
            "category": "preference.tech.python",
            "tags": ["python", "dislike"],
        },
    ]
    for item in direct_cases:
        memory_id = create_memory(
            base_url,
            profile_id,
            owner_id,
            scope_id,
            content=item["content"],
            category=item["category"],
            tags=item["tags"],
            timeout=timeout,
        )
        direct_ids.append(memory_id)
        logger.success("direct memory inserted: {} {}", memory_id, compact(item["content"], 90))
    return direct_ids


def attempt_supersede_chain(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    timeout: int,
) -> tuple[list[str], list[str]]:
    logger.info("trying to trigger a real supersede chain through event ingestion")
    logger.warning(
        "note: current schema supports only single-chain supersede. "
        "It does not support multiple old memories being superseded by one new memory."
    )

    contents = [
        "我当前最喜欢 Rust 作为主要系统编程语言，尤其看重它的内存安全和约束清晰。",
        "我现在不再像之前那样偏爱 Rust 了，我更愿意把 C++ 作为主要系统编程语言，因为它更直接。",
        "我又改变了看法，现在我仍然更喜欢 Rust，把它当作当前首选系统编程语言。",
    ]

    created_ids: list[str] = []
    superseded_ids: list[str] = []

    for index, content in enumerate(contents, start=1):
        logger.info("event {} ingesting: {}", index, compact(content, 100))
        result = create_event(
            base_url,
            profile_id,
            owner_id,
            scope_id,
            content=content,
            timeout=timeout,
        )
        logger.info(
            "event result: created={} reinforced={} superseded={} created_ids={} superseded_ids={}",
            result.get("memories_created"),
            result.get("memories_reinforced"),
            result.get("memories_superseded"),
            result.get("created_memory_ids"),
            result.get("superseded_memory_ids"),
        )
        created_ids.extend(str(item) for item in result.get("created_memory_ids", []))
        superseded_ids.extend(str(item) for item in result.get("superseded_memory_ids", []))
        time.sleep(0.5)

    created_ids = list(dict.fromkeys(created_ids))
    superseded_ids = list(dict.fromkeys(superseded_ids))
    return created_ids, superseded_ids


def inspect_chain(
    base_url: str,
    profile_id: str,
    created_ids: list[str],
    superseded_ids: list[str],
    timeout: int,
) -> None:
    candidate_ids = list(dict.fromkeys(superseded_ids + created_ids))
    if not candidate_ids:
        logger.warning("no candidate ids were produced by event ingestion; cannot inspect chain")
        return

    logger.info("inspecting candidate memories: {}", candidate_ids)
    for memory_id in candidate_ids:
        memory = get_memory(base_url, profile_id, memory_id, timeout)
        logger.info(
            "memory id={} version={} current={} supersedes={} superseded_by={} content={}",
            memory.get("id"),
            memory.get("version_number"),
            memory.get("is_current_version"),
            memory.get("supersedes"),
            memory.get("superseded_by"),
            compact(memory.get("content"), 100),
        )
        history = get_history(base_url, profile_id, memory_id, timeout)
        log_history(history)
        resolved = resolve_current(base_url, profile_id, memory_id, timeout)
        log_resolve_result(resolved)


def main() -> int:
    setup_logger()
    timeout = 180

    owner_id = f"lineage-user-{uuid.uuid4().hex[:8]}"
    scope_id = f"lineage-scope-{uuid.uuid4().hex[:8]}"

    logger.info("base_url={}", BASE_URL)
    if CONFIG_PATH.exists():
        logger.info("config_path={}", CONFIG_PATH)

    profile_id = create_profile(BASE_URL, timeout)
    logger.success("profile created: {}", profile_id)

    seed_direct_memories(BASE_URL, profile_id, owner_id, scope_id, timeout)

    logger.info("running ordinary retrieve after direct inserts")
    direct_retrieve = retrieve(
        BASE_URL,
        profile_id,
        owner_id,
        scope_id,
        query="我现在喜欢什么编程语言？",
        timeout=timeout,
    )
    log_retrieve_result(direct_retrieve)

    created_ids, superseded_ids = attempt_supersede_chain(
        BASE_URL,
        profile_id,
        owner_id,
        scope_id,
        timeout,
    )
    inspect_chain(BASE_URL, profile_id, created_ids, superseded_ids, timeout)

    logger.info("running retrieve after chain attempt")
    final_retrieve = retrieve(
        BASE_URL,
        profile_id,
        owner_id,
        scope_id,
        query="我当前更喜欢哪种系统编程语言？",
        timeout=timeout,
    )
    log_retrieve_result(final_retrieve)

    logger.success("lineage chain test finished")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
