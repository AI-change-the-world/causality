#!/usr/bin/env python3
import json
import os
import sys
from typing import Any

import requests


BASE_URL = os.environ.get("MEMORY_SERVER_BASE_URL", "http://127.0.0.1:8080")


def prompt(label: str, default: str | None = None) -> str:
    suffix = f" [{default}]" if default else ""
    value = input(f"{label}{suffix}: ").strip()
    if value:
        return value
    if default is not None:
        return default
    return ""


def print_json(title: str, payload: Any) -> None:
    print(f"\n=== {title} ===")
    print(json.dumps(payload, ensure_ascii=False, indent=2))


def post(path: str, payload: dict[str, Any]) -> dict[str, Any]:
    response = requests.post(f"{BASE_URL}{path}", json=payload, timeout=300)
    response.raise_for_status()
    return response.json()


def print_retrieval_trace(trace: dict[str, Any], memories: list[dict[str, Any]]) -> None:
    print("\n=== Retrieval Trace ===")
    print(f"query: {trace.get('query')}")
    print(f"used_vector: {trace.get('used_vector')}")
    print(f"used_fulltext: {trace.get('used_fulltext')}")
    print(f"scope_id: {trace.get('scope_id')}")
    print(f"vector_candidate_count: {trace.get('vector_candidate_count')}")
    print(f"structured_candidate_count: {trace.get('structured_candidate_count')}")
    print(f"final_result_count: {trace.get('final_result_count')}")

    for idx, item in enumerate(memories, start=1):
        memory = item["memory"]
        print(f"\n[{idx}] memory_id={memory['id']}")
        print(f"score={item['score']:.4f} similarity={item['similarity']:.4f}")
        if item.get("text_match_score") is not None:
            print(f"text_match_score={item['text_match_score']:.4f}")
        print(f"content={memory['content']}")
        print(f"metadata={json.dumps(memory['metadata'], ensure_ascii=False)}")
        lineage_sources = item.get("merged_lineage_sources") or []
        if lineage_sources:
            print("lineage_sources:")
            for source in lineage_sources:
                print(
                    f"  - source_memory_id={source['source_memory_id']} "
                    f"path={' -> '.join(source['lineage_to_current'])}"
                )


def main() -> int:
    print("Memory Server interactive demo")
    print(f"base_url: {BASE_URL}")

    system_name = prompt("System name", "demo-system")
    system_description = prompt("Profile description")
    if not system_description:
        print("Profile description is required.", file=sys.stderr)
        return 1

    owner_id = prompt("Owner ID", "demo-user")
    scope_id = prompt("Scope ID", owner_id)

    profile = post(
        "/api/v1/systems",
        {
            "name": system_name,
            "description": system_description,
        },
    )
    print_json("Created System Profile", profile)

    confirmed = post(f"/api/v1/systems/{profile['id']}/schema/confirm", {})
    print_json("Confirmed Schema", confirmed)

    print("\n=== Chat Mode ===")
    print("输入普通文本会写入 event。")
    print("输入 `/ask 你的问题` 会做检索。")
    print("输入 `/quit` 退出。")

    while True:
        line = input("\nyou> ").strip()
        if not line:
            continue
        if line == "/quit":
            break

        if line.startswith("/ask "):
            query = line[5:].strip()
            if not query:
                print("Query is empty.")
                continue

            retrieval = post(
                f"/api/v1/systems/{profile['id']}/memories/retrieve",
                {
                    "query": query,
                    "owner_id": owner_id,
                    "scope_id": scope_id,
                    "top_k": 5,
                    "options": {
                        "use_vector": True,
                        "use_fulltext": True,
                        "highlight": True,
                        "include_evidence": True,
                        "include_history": True,
                    },
                },
            )
            print_json("Retrieve Response", retrieval)
            print_retrieval_trace(retrieval["trace"], retrieval["memories"])
            continue

        event_result = post(
            f"/api/v1/systems/{profile['id']}/events",
            {
                "owner_id": owner_id,
                "scope_id": scope_id,
                "content": line,
                "source": "conversation",
            },
        )
        print_json("Event Response", event_result)

    return 0


if __name__ == "__main__":
    raise SystemExit(main())
