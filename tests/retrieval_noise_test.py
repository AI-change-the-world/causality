#!/usr/bin/env python3
"""
Seed noisy memories and compare retrieval top-k against an all-memory context.

Run from tests/:
    python3 retrieval_noise_test.py

Or from repo root:
    python3 tests/retrieval_noise_test.py

This script intentionally uses direct memory creation instead of event ingestion,
so the dataset can be seeded quickly without spending LLM calls on every record.
"""

from __future__ import annotations

import argparse
import random
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


BASE_URL = "http://localhost:8080"


@dataclass
class Case:
    name: str
    question: str
    expected_terms: list[str]


TARGET_MEMORIES = [
    {
        "content": "用户当前更喜欢 Rust 编程语言，尤其欣赏它在系统级编程中的内存安全性和清晰约束。",
        "category": "preference.tech.programming_language",
        "tags": ["rust", "programming", "systems"],
    },
    {
        "content": "用户去年经常写 C++，但现在更愿意用 Rust 处理系统级或工具链相关任务。",
        "category": "history.tech.cpp_to_rust",
        "tags": ["cpp", "rust", "history"],
    },
    {
        "content": "用户当前不喜欢 Python 作为主要工作语言，觉得它不适合作为自己偏好的系统工具方向。",
        "category": "preference.tech.dislike_python",
        "tags": ["python", "dislike"],
    },
    {
        "content": "用户不太喜欢长期做纯 CRUD 项目，更愿意做架构、工具链、抽象设计和系统可靠性相关工作。",
        "category": "preference.work.project_type",
        "tags": ["architecture", "tooling", "crud"],
    },
    {
        "content": "用户喜欢清晰、安全、可维护的工程方案，在性能极致但难维护和性能够用但清晰安全之间更偏向后者。",
        "category": "preference.work.engineering_style",
        "tags": ["maintainability", "safety"],
    },
]


NOISE_TEMPLATES = [
    "用户喜欢在周末整理房间，最近买了新的收纳盒，颜色偏向白色和灰色。",
    "用户最近开始控制下午咖啡摄入，晚上睡眠比之前稳定一些。",
    "用户计划夏天去海边旅行，但不喜欢游客特别多的热门景区。",
    "用户买菜时经常选择番茄、鸡蛋、青菜和牛肉，喜欢简单快速的晚餐。",
    "用户偏好安静的咖啡馆，不喜欢音乐声音太大的空间。",
    "用户最近在看科幻小说，喜欢世界观完整但节奏不要太拖沓的作品。",
    "用户想买一把人体工学椅，希望腰部支撑好，不要太软。",
    "用户对会议比较谨慎，喜欢会前有明确议程和产出目标。",
    "用户最近学习摄影，偏好自然光和低饱和度色彩。",
    "用户想减少手机通知，希望只保留工作和家人的高优先级提醒。",
    "用户喜欢清淡口味，外卖更常选粥、面和蒸菜。",
    "用户对运动的偏好是散步和轻量力量训练，不太喜欢高强度间歇。",
    "用户家里的网络设备放在客厅，偶尔会因为信号覆盖不均匀而烦恼。",
    "用户偏好简单的桌面布置，不喜欢过多摆件影响注意力。",
    "用户看电影时更喜欢剧情扎实的作品，不太喜欢纯视觉特效片。",
]


QUESTION_CASES = [
    Case(
        name="programming-language",
        question="用户喜欢什么编程语言？不喜欢什么语言？",
        expected_terms=["Rust", "Python"],
    ),
    Case(
        name="work-type",
        question="给用户推荐技术工作时，适合什么方向，应该避开什么？",
        expected_terms=["架构", "工具链", "CRUD"],
    ),
    Case(
        name="engineering-style",
        question="用户偏好的工程方案风格是什么？",
        expected_terms=["清晰", "安全", "可维护"],
    ),
]


def setup_logger() -> None:
    logger.remove()
    logger.add(
        sys.stderr,
        colorize=True,
        backtrace=False,
        diagnose=False,
        format="<green>{time:HH:mm:ss}</green> | <level>{level: <8}</level> | <cyan>{message}</cyan>",
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


def compact(text: Any, limit: int = 180) -> str:
    value = " ".join(str(text or "").split())
    return value if len(value) <= limit else value[: limit - 3] + "..."


def create_profile(base_url: str, timeout: int) -> str:
    payload = {
        "name": f"retrieval-noise-test-{uuid.uuid4().hex[:8]}",
        "description": (
            "用于测试个人记忆检索能力的系统。记忆中会混入大量生活、消费、旅行、"
            "健康、技术偏好等内容，检索应优先返回和问题相关的用户偏好与事实。"
        ),
    }
    response = request_json("POST", f"{base_url}/api/v1/systems", payload=payload, timeout=timeout)
    return response["id"]


def create_memory(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    content: str,
    category: str,
    tags: list[str],
    timeout: int,
) -> str:
    payload = {
        "owner_id": owner_id,
        "scope_id": scope_id,
        "content": content,
        "category": category,
        "tags": tags,
        "importance": 0.7,
        "confidence": 0.95,
        "is_global": False,
    }
    response = request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/memories",
        payload=payload,
        timeout=timeout,
    )
    return response["id"]


def seed_memories(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    noise_count: int,
    timeout: int,
) -> list[dict[str, Any]]:
    seeded: list[dict[str, Any]] = []

    logger.info("seeding target memories: {}", len(TARGET_MEMORIES))
    for item in TARGET_MEMORIES:
        memory_id = create_memory(
            base_url,
            profile_id,
            owner_id,
            scope_id,
            item["content"],
            item["category"],
            item["tags"],
            timeout,
        )
        seeded.append({**item, "id": memory_id, "kind": "target"})
        logger.success("target memory inserted: {} {}", memory_id, compact(item["content"], 90))

    logger.info("seeding noise memories: {}", noise_count)
    for index in range(noise_count):
        template = random.choice(NOISE_TEMPLATES)
        content = f"{template} 这是第 {index + 1} 条噪声记忆，用于检验检索是否能避开无关上下文。"
        category = f"noise.{index % 12:02d}"
        tags = ["noise", f"group-{index % 12}"]
        memory_id = create_memory(
            base_url,
            profile_id,
            owner_id,
            scope_id,
            content,
            category,
            tags,
            timeout,
        )
        seeded.append(
            {
                "id": memory_id,
                "kind": "noise",
                "content": content,
                "category": category,
                "tags": tags,
            }
        )
        if (index + 1) % 10 == 0 or index + 1 == noise_count:
            logger.info("noise progress: {}/{}", index + 1, noise_count)

    return seeded


def retrieve(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    question: str,
    top_k: int,
    timeout: int,
) -> dict[str, Any]:
    payload = {
        "query": question,
        "owner_id": owner_id,
        "scope_id": scope_id,
        "top_k": top_k,
        "options": {
            "include_evidence": False,
            "include_history": False,
            "use_vector": True,
            "use_fulltext": True,
            "highlight": True,
        },
    }
    return request_json(
        "POST",
        f"{base_url}/api/v1/systems/{profile_id}/memories/retrieve",
        payload=payload,
        timeout=timeout,
    )


def contains_expected(text: str, expected_terms: list[str]) -> list[str]:
    return [term for term in expected_terms if term.lower() in text.lower()]


def run_cases(
    base_url: str,
    profile_id: str,
    owner_id: str,
    scope_id: str,
    all_memories: list[dict[str, Any]],
    top_k: int,
    timeout: int,
) -> None:
    all_context = "\n".join(memory["content"] for memory in all_memories)
    all_chars = len(all_context)
    logger.info(
        "all-memory baseline: memories={} chars={} approx_tokens={}",
        len(all_memories),
        all_chars,
        all_chars // 2,
    )

    for case in QUESTION_CASES:
        logger.info("case [{}] question={}", case.name, case.question)
        started = time.time()
        result = retrieve(
            base_url,
            profile_id,
            owner_id,
            scope_id,
            case.question,
            top_k,
            timeout,
        )
        latency = time.time() - started
        hits = result.get("memories") or []
        retrieved_context = "\n".join(
            str(item.get("memory", {}).get("content", "")) for item in hits
        )
        retrieved_chars = len(retrieved_context)
        matched = contains_expected(retrieved_context, case.expected_terms)

        logger.info(
            "retrieval: candidates={} returned={} latency={:.1f}s chars={} approx_tokens={} expected_hit={}/{}",
            result.get("total_candidates"),
            len(hits),
            latency,
            retrieved_chars,
            retrieved_chars // 2,
            len(matched),
            len(case.expected_terms),
        )
        logger.info(
            "context reduction: {:.1f}x smaller than all-memory context",
            all_chars / max(retrieved_chars, 1),
        )

        target_ids = {memory["id"] for memory in all_memories if memory["kind"] == "target"}
        for index, item in enumerate(hits, start=1):
            memory = item.get("memory", {})
            kind = "target" if memory.get("id") in target_ids else "noise"
            logger.info(
                "hit #{} kind={} score={:.4f} category={} content={}",
                index,
                kind,
                item.get("score", 0.0),
                memory.get("category"),
                compact(memory.get("content"), 220),
            )

        if len(matched) == len(case.expected_terms):
            logger.success("case [{}] PASS matched={}", case.name, matched)
        else:
            missing = [term for term in case.expected_terms if term not in matched]
            logger.warning("case [{}] WEAK missing={} matched={}", case.name, missing, matched)


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Seed noisy memories and test retrieval quality.")
    parser.add_argument("--base-url", default=BASE_URL)
    parser.add_argument("--profile-id")
    parser.add_argument("--owner-id", default=f"noise-user-{uuid.uuid4().hex[:8]}")
    parser.add_argument("--scope-id", default=f"noise-scope-{uuid.uuid4().hex[:8]}")
    parser.add_argument("--noise-count", type=int, default=80)
    parser.add_argument("--top-k", type=int, default=6)
    parser.add_argument("--timeout", type=int, default=180)
    parser.add_argument("--seed", type=int, default=7)
    return parser.parse_args()


def main() -> int:
    setup_logger()
    args = parse_args()
    random.seed(args.seed)

    base_url = args.base_url.rstrip("/")
    logger.info("base_url={}", base_url)
    health = request_json("GET", f"{base_url}/health", timeout=args.timeout)
    logger.info("server health={}", health.get("status"))

    profile_id = args.profile_id or create_profile(base_url, args.timeout)
    logger.info("profile_id={} owner_id={} scope_id={}", profile_id, args.owner_id, args.scope_id)

    all_memories = seed_memories(
        base_url,
        profile_id,
        args.owner_id,
        args.scope_id,
        args.noise_count,
        args.timeout,
    )
    run_cases(
        base_url,
        profile_id,
        args.owner_id,
        args.scope_id,
        all_memories,
        args.top_k,
        args.timeout,
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
