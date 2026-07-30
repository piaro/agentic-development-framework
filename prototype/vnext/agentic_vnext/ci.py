"""CI entry point that evaluates a clean clone through the Application service."""

from __future__ import annotations

from dataclasses import dataclass
from pathlib import Path
from typing import Any

from .application import Application
from .config import load_project_config
from .explain import ExplainReport
from .filesystem_project import FileProjectStore
from .framework_lock import load_framework_lock
from .git_repository import GitRepositoryAdapter
from .model import KernelDecision


@dataclass(frozen=True)
class CiEvaluation:
    revision: str
    state: str
    merge_allowed: bool
    decision: KernelDecision
    explanation: ExplainReport


def evaluate_clean_clone(
    project_root: str | Path,
    change_id: str,
    rule_source: dict[str, Any],
) -> CiEvaluation:
    """Rebuild current state from tracked records and a clean Git revision."""

    root = Path(project_root).resolve()
    config = load_project_config(root)
    git_repository = GitRepositoryAdapter(
        root,
        config.repository_observation,
        require_clean=True,
    )
    repository = git_repository.observe()
    store = FileProjectStore(
        root,
        repository,
        contract_root=config.contract_root,
        decision_root=config.decision_root,
    )
    git_repository.assert_tracked(
        [
            ".agentic/config.yaml",
            ".agentic/framework.lock",
            *(
                path.relative_to(root)
                for path in store.record_paths(change_id)
            ),
        ]
    )
    framework_lock = load_framework_lock(root / ".agentic" / "framework.lock")
    application = Application(store, rule_source, framework_lock)
    response = application.next(change_id)
    explanation = application.explain(change_id)
    return CiEvaluation(
        revision=repository["revision"],
        state=response.decision.state,
        merge_allowed=response.decision.state == "ready-to-merge",
        decision=response.decision,
        explanation=explanation,
    )
