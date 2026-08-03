"""Thin Kernel の動作仮説を検証するための隔離 Prototype。"""

from .application import Application
from .cache import DerivedCache
from .ci import CiEvaluation, evaluate_clean_clone
from .explain import ExplainReport
from .framework_lock import (
    FrameworkLock,
    build_framework_lock,
    load_framework_lock,
)
from .filesystem_project import FileProjectStore
from .git_repository import GitRepositoryAdapter
from .golden import GoldenMismatch, verify_golden_suite
from .project import InMemoryProjectStore, load_project
from .rules import load_rule_source
from .schema import SchemaValidationError, default_schema_registry

__all__ = [
    "Application",
    "DerivedCache",
    "CiEvaluation",
    "ExplainReport",
    "FrameworkLock",
    "FileProjectStore",
    "GitRepositoryAdapter",
    "GoldenMismatch",
    "InMemoryProjectStore",
    "SchemaValidationError",
    "load_project",
    "build_framework_lock",
    "load_framework_lock",
    "evaluate_clean_clone",
    "default_schema_registry",
    "load_rule_source",
    "verify_golden_suite",
]
