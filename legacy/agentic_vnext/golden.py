"""Verify checked-in, language-neutral compatibility fixtures."""

from __future__ import annotations

from copy import deepcopy
from dataclasses import asdict
import json
from pathlib import Path
from typing import Any

from .application import Application
from .context import ContextCompiler
from .detection import TypedFactDetector
from .explain import EXPLAIN_REPORT_SCHEMA_VERSION
from .framework_lock import build_framework_lock, validate_framework_lock
from .filesystem_project import (
    FILESYSTEM_PROJECT_PROTOCOL_VERSION,
    FileProjectStore,
)
from .kernel import ThinKernel
from .model import (
    KernelDecision,
    NextAction,
    ProjectSnapshot,
    RequirementInstance,
    canonical_digest,
    canonical_json,
)
from .project import InMemoryProjectStore, build_project_snapshot
from .rules import compile_rule_index
from .schema import (
    SchemaValidationError,
    default_schema_registry,
    validate_json_document,
)
from .submission import RESULT_SUBMISSION_PROTOCOL_VERSION, prepare_result
from .versions import (
    APPLICATION_PROTOCOL_VERSION,
    CANONICALIZATION_VERSION,
    CONTEXT_COMPILER_VERSION,
    DETECTOR_ID,
    DETECTOR_VERSION,
    FRAMEWORK_LOCK_SCHEMA_VERSION,
    KERNEL_VERSION,
    PROJECT_SNAPSHOT_PROTOCOL_VERSION,
    RULE_COMPILER_VERSION,
)


class GoldenMismatch(AssertionError):
    """The runtime no longer matches a reviewed compatibility fixture."""


def load_golden(path: str | Path) -> dict[str, Any]:
    with Path(path).open(encoding="utf-8") as stream:
        value = json.load(stream)
    if not isinstance(value, dict):
        raise ValueError(f"golden fixture must be an object: {path}")
    return value


def verify_golden_suite(root: str | Path) -> None:
    """Verify every v1 boundary without rewriting expected values."""

    golden_root = Path(root)
    manifest = load_golden(golden_root / "manifest.json")
    _expect_equal(
        "golden suite id",
        "agentic-vnext-golden-v1",
        manifest.get("suite_id"),
    )
    verifiers = {
        "canonicalization": verify_canonicalization_case_set,
        "schema-validation": verify_schema_case_set,
        "rule-compilation": verify_rule_compilation_case_set,
        "typed-fact-detection": verify_detection_case_set,
        "kernel-decision": verify_kernel_case_set,
        "context-compilation": verify_context_case_set,
        "project-snapshot": lambda case: verify_project_snapshot_case_set(
            case,
            golden_root,
        ),
        "framework-lock": lambda case: verify_framework_lock_case_set(
            case,
            golden_root,
        ),
        "result-submission": lambda case: verify_result_submission_case_set(
            case,
            golden_root,
        ),
        "filesystem-project": lambda case: verify_filesystem_project_case_set(
            case,
            golden_root,
        ),
        "persistent-application": lambda case: verify_persistent_application_case_set(
            case,
            golden_root,
        ),
        "explain-report": lambda case: verify_explain_report_case_set(
            case,
            golden_root,
        ),
        "application": verify_application_case,
        "application-scenario": lambda case: verify_application_scenario(
            case,
            golden_root,
        ),
    }
    seen_kinds: set[str] = set()
    seen_paths: set[str] = set()
    for entry in manifest.get("cases", []):
        kind = entry["kind"]
        relative_path = entry["path"]
        if kind not in verifiers:
            raise GoldenMismatch(f"unsupported golden case kind: {kind}")
        if relative_path in seen_paths:
            raise GoldenMismatch(f"duplicate golden case path: {relative_path}")
        seen_kinds.add(kind)
        seen_paths.add(relative_path)
        verifiers[kind](
            load_golden(_resolve_inside(golden_root, relative_path))
        )
    missing_kinds = set(verifiers) - seen_kinds
    if missing_kinds:
        raise GoldenMismatch(
            "missing golden case kinds: " + ", ".join(sorted(missing_kinds))
        )


def verify_canonicalization_case_set(case_set: dict[str, Any]) -> None:
    _expect_equal(
        "canonicalization version",
        CANONICALIZATION_VERSION,
        case_set.get("canonicalization_version"),
    )
    _expect_equal("number scope", "integers-only", case_set.get("number_scope"))
    for case in case_set.get("cases", []):
        case_id = case["case_id"]
        if _contains_float(case["value"]):
            raise GoldenMismatch(
                f"{case_id}: floating-point value is outside canonical-json-v1"
            )
        _expect_equal(
            f"{case_id} canonical JSON",
            case["canonical_json"],
            canonical_json(case["value"]),
        )
        _expect_equal(
            f"{case_id} digest",
            case["digest"],
            canonical_digest(case["value"]),
        )
    for case in case_set.get("invalid_cases", []):
        try:
            canonical_json(case["value"])
        except ValueError as error:
            if case["error"] not in str(error):
                raise GoldenMismatch(
                    f"{case['case_id']}: unexpected error {error}"
                ) from error
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected canonicalization failure"
            )


def verify_schema_case_set(case_set: dict[str, Any]) -> None:
    registry = default_schema_registry()
    _expect_equal(
        "Schema bundle digest",
        case_set.get("schema_bundle_digest"),
        registry.digest,
    )
    for case in case_set.get("cases", []):
        case_id = case["case_id"]
        try:
            registry.validate(case["record_kind"], case["record"])
        except SchemaValidationError as error:
            if case["valid"]:
                raise GoldenMismatch(
                    f"{case_id}: expected valid, got {error}"
                ) from error
            expected_path = case["error_path"]
            if f" at {expected_path}:" not in str(error):
                raise GoldenMismatch(
                    f"{case_id}: expected error at {expected_path}, got {error}"
                ) from error
        else:
            if not case["valid"]:
                raise GoldenMismatch(f"{case_id}: expected invalid, got valid")


def verify_rule_compilation_case_set(case_set: dict[str, Any]) -> None:
    """Lock the normalized Rule Index and deterministic configuration errors."""

    _expect_equal(
        "Rule Compiler version",
        RULE_COMPILER_VERSION,
        case_set.get("compiler_version"),
    )
    registry = default_schema_registry()
    _expect_equal(
        "Rule Compiler Schema bundle digest",
        case_set.get("schema_bundle_digest"),
        registry.digest,
    )
    for case in case_set.get("cases", []):
        for variant_index, source in enumerate(case.get("source_variants", [])):
            actual = _rule_index_value(compile_rule_index(source))
            _expect_equal(
                f"{case['case_id']} variant {variant_index}",
                case["expected"],
                actual,
            )
    for case in case_set.get("invalid_cases", []):
        try:
            compile_rule_index(case["source"])
        except (KeyError, TypeError, ValueError) as error:
            _expect_equal(
                f"{case['case_id']} error",
                case["error"],
                str(error),
            )
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected Rule compilation failure"
            )


def _rule_index_value(rule_index: Any) -> dict[str, Any]:
    """Represent the internal index without Python dict/tuple distinctions."""

    return {
        "requirements": [
            {
                "id": item.id,
                "phase": item.phase,
                "role": item.role,
                "result_schema": item.result_schema,
                "depends_on": list(item.depends_on),
                "context": list(item.context),
                "definition_digest": item.definition_digest,
            }
            for item in sorted(
                rule_index.requirements.values(),
                key=lambda value: value.id,
            )
        ],
        "rules": [
            {
                "id": item.id,
                "requirement": item.requirement_id,
                "condition": item.condition,
                "signal": item.signal,
                "repository_phase": item.repository_phase,
                "subjects": list(item.subjects),
            }
            for item in sorted(rule_index.rules, key=lambda value: value.id)
        ],
        "digest": rule_index.digest,
    }


def verify_detection_case_set(case_set: dict[str, Any]) -> None:
    """Lock candidate identities without coupling the fixture to a Store."""

    _expect_equal("Detector ID", DETECTOR_ID, case_set.get("detector_id"))
    _expect_equal(
        "Detector version",
        DETECTOR_VERSION,
        case_set.get("detector_version"),
    )
    detector = TypedFactDetector()
    for case in case_set.get("cases", []):
        for variant_index, detector_input in enumerate(
            case.get("input_variants", [])
        ):
            snapshot = ProjectSnapshot(
                change_id=detector_input["change_id"],
                change={},
                contracts=(),
                decisions=(),
                results=(),
                evidence=(),
                repository={
                    "facts": deepcopy(detector_input["facts"]),
                    "coverage": deepcopy(detector_input["coverage"]),
                },
                artifact_digests=deepcopy(
                    detector_input.get("artifact_digests", {})
                ),
                digest="",
            )
            # canonical JSON removes Python tuple/list representation details.
            actual = json.loads(
                canonical_json(asdict(detector.detect(snapshot)))
            )
            _expect_equal(
                f"{case['case_id']} variant {variant_index}",
                case["expected"],
                actual,
            )
    for case in case_set.get("invalid_cases", []):
        detector_input = case["input"]
        snapshot = ProjectSnapshot(
            change_id=detector_input["change_id"],
            change={},
            contracts=(),
            decisions=(),
            results=(),
            evidence=(),
            repository={
                "facts": deepcopy(detector_input["facts"]),
                "coverage": deepcopy(detector_input["coverage"]),
            },
            artifact_digests=deepcopy(
                detector_input.get("artifact_digests", {})
            ),
            digest="",
        )
        try:
            detector.detect(snapshot)
        except ValueError as error:
            _expect_equal(
                f"{case['case_id']} error",
                case["error"],
                str(error),
            )
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected Detector failure"
            )


def verify_kernel_case_set(case_set: dict[str, Any]) -> None:
    """Verify the pure Kernel independently of persistence and Context output."""

    _expect_equal("Kernel version", KERNEL_VERSION, case_set.get("kernel_version"))
    common = case_set["common"]
    rule_index = compile_rule_index(common["rule_source"])
    detector = TypedFactDetector()
    kernel = ThinKernel()
    for case in case_set.get("cases", []):
        kernel_input = case["input"]
        snapshot = ProjectSnapshot(
            change_id=common["change_id"],
            change={},
            contracts=tuple(deepcopy(kernel_input.get("contracts", []))),
            decisions=tuple(deepcopy(kernel_input.get("decisions", []))),
            results=tuple(
                deepcopy(common["result_records"][record_id])
                for record_id in kernel_input.get("result_refs", [])
            ),
            evidence=(),
            repository={
                "phase": kernel_input["repository_phase"],
                "facts": deepcopy(common["facts"]),
                "coverage": deepcopy(
                    kernel_input.get("coverage", common["coverage"])
                ),
            },
            artifact_digests=deepcopy(common["artifact_digests"]),
            digest="",
        )
        detection = detector.detect(snapshot)
        decision = kernel.evaluate(snapshot, rule_index, detection)
        _expect_equal(
            f"{case['case_id']} decision",
            case["expected"],
            _kernel_checkpoint(decision),
        )


def _kernel_checkpoint(decision: Any) -> dict[str, Any]:
    decision_body = decision.as_dict()
    action = decision.action
    return {
        "state": decision.state,
        "action": (
            {
                "id": action.id,
                "role": action.role,
                "action": action.action,
                "result_schema": action.expected_result_schema,
                "instance_keys": [
                    item.instance_key
                    for item in action.requirement_instances
                ],
                "candidate_fingerprints": list(
                    action.candidate_fingerprints
                ),
            }
            if action is not None
            else None
        ),
        "instance_statuses": {
            item.instance_key: item.status
            for item in decision.requirement_instances
        },
        "diagnostics": list(decision.diagnostics),
        "decision_digest": canonical_digest(decision_body),
    }


def verify_context_case_set(case_set: dict[str, Any]) -> None:
    """Verify source selection and Generated Context identity."""

    _expect_equal(
        "Context Compiler version",
        CONTEXT_COMPILER_VERSION,
        case_set.get("context_compiler_version"),
    )
    common = case_set["common"]
    snapshot_source = common["snapshot"]
    snapshot = ProjectSnapshot(
        change_id=snapshot_source["change_id"],
        change=deepcopy(snapshot_source["change"]),
        contracts=tuple(deepcopy(snapshot_source["contracts"])),
        decisions=tuple(deepcopy(snapshot_source["decisions"])),
        results=tuple(deepcopy(snapshot_source["results"])),
        evidence=tuple(deepcopy(snapshot_source["evidence"])),
        repository=deepcopy(snapshot_source["repository"]),
        artifact_digests=deepcopy(snapshot_source["artifact_digests"]),
        digest="",
    )
    detection = TypedFactDetector().detect(snapshot)
    compiler = ContextCompiler()
    definitions = common["requirement_instances"]
    for case in case_set.get("cases", []):
        all_instances = tuple(
            RequirementInstance(**deepcopy(definitions[instance_ref]))
            for instance_ref in case.get("all_instance_refs", [])
        )
        action_source = case.get("action")
        action = None
        if action_source is not None:
            action_instances = tuple(
                RequirementInstance(**deepcopy(definitions[instance_ref]))
                for instance_ref in action_source.get("instance_refs", [])
            )
            action = NextAction(
                id=action_source["id"],
                role=action_source["role"],
                action=action_source["action"],
                requirement_instances=action_instances,
                reason=action_source["reason"],
                expected_result_schema=action_source["expected_result_schema"],
                candidate_fingerprints=tuple(
                    action_source.get("candidate_fingerprints", [])
                ),
            )
        decision = KernelDecision(
            state=case["state"],
            action=action,
            requirement_instances=all_instances,
        )
        actual = compiler.compile(decision, snapshot, detection)
        _expect_equal(
            f"{case['case_id']} context",
            case["expected"],
            _context_checkpoint(actual),
        )


def _context_checkpoint(context: Any) -> dict[str, Any] | None:
    if context is None:
        return None
    return {
        "action_id": context.action_id,
        "role": context.role,
        "source_refs": list(context.source_refs),
        "instance_source_refs": {
            instance_key: list(source_digests)
            for instance_key, source_digests
            in context.instance_source_digests.items()
        },
        "signal_candidate_fingerprints": [
            candidate["fingerprint"]
            for candidate in context.payload["signal_candidates"]
        ],
        "digest": context.digest,
    }


def verify_project_snapshot_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Verify Project normalization independently of storage layout."""

    _expect_equal(
        "Project Snapshot protocol version",
        PROJECT_SNAPSHOT_PROTOCOL_VERSION,
        case_set.get("snapshot_protocol_version"),
    )
    root = Path(golden_root)
    base = load_golden(_resolve_inside(root, case_set["base_case"]))
    base_project = base["input"]["project"]
    for case in case_set.get("cases", []):
        project = deepcopy(base_project)
        if case.get("reverse_record_collections"):
            for collection in (
                "changes",
                "contracts",
                "decisions",
                "results",
                "evidence",
            ):
                project[collection] = list(
                    reversed(project.get(collection, []))
                )
        if case.get("append_unrelated_records"):
            for collection, records in case_set[
                "unrelated_records"
            ].items():
                project.setdefault(collection, []).extend(deepcopy(records))
        if case.get("append_multi_change_records"):
            for collection, records in case_set[
                "multi_change_records"
            ].items():
                project.setdefault(collection, []).extend(deepcopy(records))
        snapshot = build_project_snapshot(project, case["change_id"])
        _expect_equal(
            f"{case['case_id']} snapshot",
            case.get("expected", case_set["expected"]),
            _project_snapshot_checkpoint(snapshot),
        )
    for case in case_set.get("invalid_cases", []):
        try:
            build_project_snapshot(deepcopy(base_project), case["change_id"])
        except (KeyError, TypeError, ValueError) as error:
            _expect_equal(
                f"{case['case_id']} error",
                case["error"],
                str(error),
            )
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected Project Snapshot failure"
            )


def _project_snapshot_checkpoint(snapshot: ProjectSnapshot) -> dict[str, Any]:
    return {
        "change_id": snapshot.change_id,
        "contract_ids": [item["id"] for item in snapshot.contracts],
        "decision_ids": [item["id"] for item in snapshot.decisions],
        "result_ids": [item["id"] for item in snapshot.results],
        "evidence_ids": [item["id"] for item in snapshot.evidence],
        "repository_phase": snapshot.repository.get("phase", "pre-build"),
        "artifact_digests": snapshot.artifact_digests,
        "digest": snapshot.digest,
    }


def verify_framework_lock_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Verify that partial Framework upgrades stop before evaluation."""

    _expect_equal(
        "Framework lock Schema version",
        FRAMEWORK_LOCK_SCHEMA_VERSION,
        case_set.get("framework_lock_schema_version"),
    )
    root = Path(golden_root)
    base = load_golden(_resolve_inside(root, case_set["base_case"]))
    rule_source = base["input"]["rule_source"]
    reviewed_lock = base["input"]["framework_lock"]
    rule_index = compile_rule_index(rule_source)
    _expect_equal(
        "built Framework lock",
        reviewed_lock,
        build_framework_lock(rule_source, rule_index),
    )
    validated = validate_framework_lock(
        reviewed_lock,
        rule_source,
        rule_index,
    )
    _expect_equal(
        "Framework lock digest",
        case_set["expected_digest"],
        validated.digest,
    )

    for case in case_set.get("invalid_cases", []):
        mutated = deepcopy(reviewed_lock)
        _mutate_path(
            mutated,
            case["operation"],
            case["path"],
            case.get("value"),
        )
        try:
            validate_framework_lock(mutated, rule_source, rule_index)
        except ValueError as error:
            if case["error_path"] not in str(error):
                raise GoldenMismatch(
                    f"{case['case_id']}: expected error path "
                    f"{case['error_path']!r}, got {error}"
                ) from error
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected Framework lock failure"
            )


def _mutate_path(
    value: Any,
    operation: str,
    path: list[str | int],
    replacement: Any,
) -> None:
    parent = value
    for field in path[:-1]:
        parent = parent[field]
    field = path[-1]
    if operation in {"set", "add"}:
        parent[field] = replacement
    elif operation == "remove":
        del parent[field]
    else:
        raise GoldenMismatch(f"unsupported fixture mutation: {operation}")


def verify_result_submission_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Verify pure submission validation separately from persistence."""

    _expect_equal(
        "Result submission protocol version",
        RESULT_SUBMISSION_PROTOCOL_VERSION,
        case_set.get("result_submission_protocol_version"),
    )
    root = Path(golden_root)
    base = load_golden(_resolve_inside(root, case_set["base_case"]))
    scenario = load_golden(
        _resolve_inside(root, case_set["scenario_case"])
    )
    step = scenario["steps"][case_set["step_index"]]["input"]
    project = deepcopy(base["input"]["project"])
    change_id = base["change_id"]
    snapshot = build_project_snapshot(project, change_id)
    rule_index = compile_rule_index(base["input"]["rule_source"])
    detection = TypedFactDetector().detect(snapshot)
    decision = ThinKernel().evaluate(snapshot, rule_index, detection)
    context = ContextCompiler().compile(decision, snapshot, detection)
    if context is None or decision.action is None:
        raise GoldenMismatch("Result submission base case produced no Action")
    submission = {
        "change_id": change_id,
        "action_id": decision.action.id,
        "context_digest": context.digest,
        "role": decision.action.role,
        "result_schema": decision.action.expected_result_schema,
        "payload": deepcopy(step["payload"]),
        "output_refs": deepcopy(step.get("output_refs", [])),
    }
    actual = prepare_result(
        context=context,
        current=snapshot,
        output_refs=tuple(submission["output_refs"]),
        **{key: value for key, value in submission.items() if key != "output_refs"},
    )
    _expect_equal("prepared Result Record", case_set["expected"], actual)

    for case in case_set.get("invalid_cases", []):
        mutated = {
            "project": deepcopy(project),
            "submission": deepcopy(submission),
        }
        _mutate_path(
            mutated,
            case["operation"],
            case["path"],
            case.get("value"),
        )
        current = build_project_snapshot(mutated["project"], change_id)
        submitted = mutated["submission"]
        try:
            prepare_result(
                context=context,
                current=current,
                output_refs=tuple(submitted["output_refs"]),
                **{
                    key: value
                    for key, value in submitted.items()
                    if key != "output_refs"
                },
            )
        except (KeyError, TypeError, ValueError) as error:
            if case["error"] not in str(error):
                raise GoldenMismatch(
                    f"{case['case_id']}: expected error containing "
                    f"{case['error']!r}, got {error}"
                ) from error
        else:
            raise GoldenMismatch(
                f"{case['case_id']}: expected Result submission failure"
            )


def verify_filesystem_project_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Verify Git-managed Record layout and persistence invariants."""

    import tempfile

    _expect_equal(
        "Filesystem Project protocol version",
        FILESYSTEM_PROJECT_PROTOCOL_VERSION,
        case_set.get("filesystem_project_protocol_version"),
    )
    root = Path(golden_root)
    base = load_golden(_resolve_inside(root, case_set["base_case"]))
    result_case = load_golden(
        _resolve_inside(root, case_set["result_case"])
    )
    scenario = load_golden(
        _resolve_inside(root, case_set["scenario_case"])
    )
    project = base["input"]["project"]
    repository = project["repository"]
    change_id = base["change_id"]
    result = result_case["expected"]
    decision = scenario["steps"][case_set["decision_step"]]["input"]
    contract = scenario["steps"][case_set["contract_step"]]["input"]

    for format_case in case_set["formats"]:
        document_format = format_case["document_format"]
        with tempfile.TemporaryDirectory() as temporary:
            project_root = Path(temporary).resolve()
            store = FileProjectStore.initialize(
                project_root,
                project,
                document_format=document_format,
            )
            _expect_equal(
                f"{document_format} initial Snapshot digest",
                case_set["expected_initial_snapshot_digest"],
                store.snapshot(change_id).digest,
            )
            # Physical default roots are Rust implementation details after the
            # vNext implementation language was consolidated. The legacy
            # Python Store still verifies persistence semantics, not path parity.

            # Initialization must preflight all targets and preserve existing data.
            try:
                FileProjectStore.initialize(
                    project_root,
                    project,
                    document_format=document_format,
                )
            except ValueError as error:
                if "would overwrite" not in str(error):
                    raise GoldenMismatch(
                        f"{document_format}: unexpected initialization error: "
                        f"{error}"
                    ) from error
            else:
                raise GoldenMismatch(
                    f"{document_format}: expected initialization conflict"
                )
            _expect_equal(
                f"{document_format} Snapshot after initialization conflict",
                case_set["expected_initial_snapshot_digest"],
                store.snapshot(change_id).digest,
            )

            extension = "md" if document_format == "markdown" else "yaml"
            prose_path = store.contract_root / (
                f"contract.order-lifecycle.{extension}"
            )
            if document_format == "markdown":
                text = prose_path.read_text(encoding="utf-8")
                prose = case_set["markdown_prose"]
                prose_path.write_text(
                    text.replace(
                        "```agentic-contract\n",
                        prose + "\n\n```agentic-contract\n",
                        1,
                    ),
                    encoding="utf-8",
                )

            store.append_result(result)
            _expect_equal(
                f"{document_format} Result Snapshot digest",
                case_set["expected_result_snapshot_digest"],
                store.snapshot(change_id).digest,
            )
            concurrent = FileProjectStore(project_root, repository)
            try:
                concurrent.append_result(result)
            except ValueError as error:
                if "record already exists" not in str(error):
                    raise GoldenMismatch(
                        f"{document_format}: unexpected duplicate Result error: "
                        f"{error}"
                    ) from error
            else:
                raise GoldenMismatch(
                    f"{document_format}: duplicate Result was accepted"
                )

            store.upsert_decision(decision)
            store.upsert_contract(contract)
            _expect_equal(
                f"{document_format} updated Snapshot digest",
                case_set["expected_updated_snapshot_digest"],
                store.snapshot(change_id).digest,
            )
            if document_format == "markdown" and case_set["markdown_prose"] not in (
                prose_path.read_text(encoding="utf-8")
            ):
                raise GoldenMismatch(
                    "Markdown prose was removed by Contract update"
                )
            temporary_files = list(project_root.rglob("*.tmp"))
            if temporary_files:
                raise GoldenMismatch(
                    "atomic update left temporary files: "
                    + ", ".join(str(path) for path in temporary_files)
                )
            restarted = FileProjectStore(project_root, repository)
            _expect_equal(
                f"{document_format} restarted Snapshot digest",
                case_set["expected_updated_snapshot_digest"],
                restarted.snapshot(change_id).digest,
            )

            concurrency = case_set["shared_contract_concurrency"]
            initial_shared = deepcopy(concurrency["initial"])
            store.upsert_contract(initial_shared)
            initial_digest = canonical_digest(initial_shared)
            first_writer = FileProjectStore(project_root, repository)
            stale_writer = FileProjectStore(project_root, repository)
            first_writer.upsert_contract(
                deepcopy(concurrency["first_update"]),
                initial_digest,
            )
            try:
                stale_writer.upsert_contract(
                    deepcopy(concurrency["stale_update"]),
                )
            except ValueError as error:
                if concurrency["missing_digest_error"] not in str(error):
                    raise GoldenMismatch(
                        f"{document_format}: unexpected missing digest error: "
                        f"{error}"
                    ) from error
            else:
                raise GoldenMismatch(
                    f"{document_format}: Shared Contract update without digest "
                    "was accepted"
                )
            try:
                stale_writer.upsert_contract(
                    deepcopy(concurrency["stale_update"]),
                    initial_digest,
                )
            except ValueError as error:
                if concurrency["stale_error"] not in str(error):
                    raise GoldenMismatch(
                        f"{document_format}: unexpected stale update error: "
                        f"{error}"
                    ) from error
            else:
                raise GoldenMismatch(
                    f"{document_format}: stale Shared Contract update was accepted"
                )
            current_shared = next(
                contract
                for contract in store.snapshot(change_id).contracts
                if contract["id"] == initial_shared["id"]
            )
            _expect_equal(
                f"{document_format} Shared Contract after stale update",
                concurrency["expected_text"],
                current_shared["clauses"][0]["text"],
            )

    with tempfile.TemporaryDirectory() as temporary:
        for invalid in case_set["invalid_source_roots"]:
            try:
                FileProjectStore(
                    temporary,
                    repository,
                    contract_root=invalid["contract_root"],
                )
            except ValueError as error:
                if invalid["error"] not in str(error):
                    raise GoldenMismatch(
                        f"{invalid['case_id']}: expected "
                        f"{invalid['error']!r}, got {error}"
                    ) from error
            else:
                raise GoldenMismatch(
                    f"{invalid['case_id']}: unsafe source root was accepted"
                )


def verify_persistent_application_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Replay the lifecycle while recreating the Application at checkpoints."""

    import tempfile

    _expect_equal(
        "Persistent Application protocol version",
        APPLICATION_PROTOCOL_VERSION,
        case_set.get("persistent_application_protocol_version"),
    )
    root = Path(golden_root)
    scenario = load_golden(
        _resolve_inside(root, case_set["scenario_case"])
    )
    base = load_golden(
        _resolve_inside(root, scenario["base_case"])
    )
    project = deepcopy(base["input"]["project"])
    rule_source = deepcopy(base["input"]["rule_source"])
    framework_lock = deepcopy(base["input"]["framework_lock"])
    change_id = base["change_id"]

    for document_format in case_set["document_formats"]:
        with tempfile.TemporaryDirectory() as temporary:
            current_repository = deepcopy(project["repository"])
            store = FileProjectStore.initialize(
                temporary,
                project,
                document_format=document_format,
            )
            app = Application(store, rule_source, framework_lock)
            response = app.next(change_id)
            checkpoint_count = 0

            for index, step in enumerate(scenario["steps"]):
                operation = step["operation"]
                if operation == "submit-current":
                    if response.context is None or response.decision.action is None:
                        raise GoldenMismatch(
                            f"{document_format} step {index}: no current Action"
                        )
                    action = response.decision.action
                    response = app.submit(
                        change_id=change_id,
                        action_id=action.id,
                        context_digest=response.context.digest,
                        role=action.role,
                        result_schema=action.expected_result_schema,
                        payload=deepcopy(step["input"]["payload"]),
                        output_refs=tuple(
                            step["input"].get("output_refs", [])
                        ),
                    )
                elif operation == "upsert-decision":
                    store.upsert_decision(deepcopy(step["input"]))
                elif operation == "upsert-contract":
                    store.upsert_contract(
                        deepcopy(step["input"]),
                        step.get("expected_digest"),
                    )
                elif operation == "update-repository":
                    current_repository = deepcopy(step["input"])
                    store.update_repository(current_repository)
                    response = app.next(change_id)
                else:
                    raise GoldenMismatch(
                        f"{document_format} step {index}: unsupported "
                        f"operation {operation!r}"
                    )

                if "expected" not in step:
                    continue
                checkpoint_count += 1
                _expect_equal(
                    f"{document_format} persistent step {index}",
                    step["expected"],
                    _scenario_checkpoint(store, change_id, response),
                )

                # Issued Actions are not persisted. A new process must recreate
                # the same Action and Context from authoritative Records.
                store = FileProjectStore(temporary, current_repository)
                app = Application(store, rule_source, framework_lock)
                response = app.next(change_id)
                _expect_equal(
                    f"{document_format} restarted step {index}",
                    step["expected"],
                    _scenario_checkpoint(store, change_id, response),
                )

            _expect_equal(
                f"{document_format} checkpoint count",
                case_set["expected_checkpoints"],
                checkpoint_count,
            )
            _expect_equal(
                f"{document_format} Result count",
                case_set["expected_result_records"],
                len(store.snapshot(change_id).results),
            )


def verify_explain_report_case_set(
    case_set: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Replay the lifecycle and verify the read-only explanation boundary."""

    _expect_equal(
        "Explain Report Schema version",
        EXPLAIN_REPORT_SCHEMA_VERSION,
        case_set.get("explain_report_schema_version"),
    )
    root = Path(golden_root).resolve()
    vnext_root = root.parents[1]
    schema_path = (root / case_set["schema_path"]).resolve()
    try:
        schema_path.relative_to(vnext_root)
    except ValueError as error:
        raise GoldenMismatch(
            f"Explain Report Schema escapes vNext root: {case_set['schema_path']}"
        ) from error
    report_schema = load_golden(schema_path)
    base = load_golden(_resolve_inside(root, case_set["base_case"]))
    scenario = load_golden(
        _resolve_inside(root, case_set["scenario_case"])
    )
    store = InMemoryProjectStore(deepcopy(base["input"]["project"]))
    app = Application(
        store,
        deepcopy(base["input"]["rule_source"]),
        deepcopy(base["input"]["framework_lock"]),
    )
    change_id = base["change_id"]
    response = app.next(change_id)
    checkpoints = {
        checkpoint["after_step"]: checkpoint
        for checkpoint in case_set["checkpoints"]
    }
    if len(checkpoints) != len(case_set["checkpoints"]):
        raise GoldenMismatch("Explain Report has duplicate checkpoints")

    def verify_checkpoint(after_step: int | None) -> None:
        expected = checkpoints.get(after_step)
        if expected is None:
            raise GoldenMismatch(
                f"Explain Report checkpoint is missing: {after_step}"
            )
        before_digest = store.snapshot(change_id).digest
        report = app.explain(change_id)
        body = report.as_dict()
        validate_json_document(body, report_schema)
        after_digest = store.snapshot(change_id).digest
        _expect_equal(
            f"Explain Report {expected['label']} read-only Snapshot",
            before_digest,
            after_digest,
        )
        actual = {
            "label": expected["label"],
            "after_step": after_step,
            "state": report.state,
            "candidate_count": len(report.candidates),
            "requirement_count": len(report.requirements),
            "authority_statuses": [
                authority["status"] for authority in report.authority
            ],
            "report_digest": canonical_digest(body),
            "text_digest": canonical_digest(report.render_text()),
        }
        _expect_equal(
            f"Explain Report {expected['label']}",
            expected,
            actual,
        )

    verify_checkpoint(None)
    for index, step in enumerate(scenario["steps"]):
        operation = step["operation"]
        if operation == "submit-current":
            if response.context is None or response.decision.action is None:
                raise GoldenMismatch(
                    f"Explain Report step {index}: no current Action"
                )
            action = response.decision.action
            response = app.submit(
                change_id=change_id,
                action_id=action.id,
                context_digest=response.context.digest,
                role=action.role,
                result_schema=action.expected_result_schema,
                payload=deepcopy(step["input"]["payload"]),
                output_refs=tuple(step["input"].get("output_refs", [])),
            )
        elif operation == "upsert-decision":
            store.upsert_decision(deepcopy(step["input"]))
        elif operation == "upsert-contract":
            store.upsert_contract(
                deepcopy(step["input"]),
                step.get("expected_digest"),
            )
        elif operation == "update-repository":
            store.update_repository(deepcopy(step["input"]))
            response = app.next(change_id)
        else:
            raise GoldenMismatch(
                f"Explain Report step {index}: unsupported operation "
                f"{operation!r}"
            )
        verify_checkpoint(index)


def verify_application_case(case: dict[str, Any]) -> None:
    project = case["input"]["project"]
    rule_source = case["input"]["rule_source"]
    framework_lock = case["input"]["framework_lock"]
    expected = case["expected"]

    store = InMemoryProjectStore(project)
    rule_index = compile_rule_index(rule_source)
    app = Application(store, rule_source, framework_lock)
    response = app.next(case["change_id"])
    snapshot = store.snapshot(case["change_id"])
    decision = response.decision.as_dict()

    actual = {
        "snapshot_digest": snapshot.digest,
        "rule_index_digest": rule_index.digest,
        "framework_lock_digest": app.framework_lock.digest,
        # JSON has arrays, not language-specific tuple/list distinctions.
        "decision": json.loads(canonical_json(decision)),
        "decision_digest": canonical_digest(decision),
        "context_digest": response.context.digest if response.context else None,
        "context_source_digests": (
            response.context.source_digests if response.context else {}
        ),
    }
    _expect_equal(f"{case['case_id']} output", expected, actual)


def verify_application_scenario(
    scenario: dict[str, Any],
    golden_root: str | Path,
) -> None:
    """Replay persisted operations and compare every state transition."""

    root = Path(golden_root)
    base = load_golden(
        _resolve_inside(root, scenario["base_case"])
    )
    project = deepcopy(base["input"]["project"])
    rule_source = deepcopy(base["input"]["rule_source"])
    framework_lock = deepcopy(base["input"]["framework_lock"])
    change_id = base["change_id"]

    store = InMemoryProjectStore(project)
    app = Application(store, rule_source, framework_lock)
    response = app.next(change_id)

    for index, step in enumerate(scenario.get("steps", [])):
        operation = step["operation"]
        if operation == "submit-current":
            if response.context is None or response.decision.action is None:
                raise GoldenMismatch(
                    f"{scenario['case_id']} step {index}: no current Action"
                )
            action = response.decision.action
            response = app.submit(
                change_id=change_id,
                action_id=action.id,
                context_digest=response.context.digest,
                role=action.role,
                result_schema=action.expected_result_schema,
                payload=deepcopy(step["input"]["payload"]),
                output_refs=tuple(step["input"].get("output_refs", [])),
            )
        elif operation == "upsert-decision":
            store.upsert_decision(deepcopy(step["input"]))
        elif operation == "upsert-contract":
            store.upsert_contract(
                deepcopy(step["input"]),
                step.get("expected_digest"),
            )
        elif operation == "update-repository":
            store.update_repository(deepcopy(step["input"]))
            response = app.next(change_id)
        else:
            raise GoldenMismatch(
                f"{scenario['case_id']} step {index}: "
                f"unsupported operation {operation!r}"
            )

        if "expected" in step:
            _expect_equal(
                f"{scenario['case_id']} step {index}",
                step["expected"],
                _scenario_checkpoint(store, change_id, response),
            )


def _scenario_checkpoint(
    store: InMemoryProjectStore,
    change_id: str,
    response: Any,
) -> dict[str, Any]:
    action = response.decision.action
    return {
        "state": response.decision.state,
        "snapshot_digest": store.snapshot(change_id).digest,
        "decision_digest": canonical_digest(response.decision.as_dict()),
        "context_digest": response.context.digest if response.context else None,
        "action": (
            {
                "id": action.id,
                "role": action.role,
                "result_schema": action.expected_result_schema,
                "instance_keys": [
                    item.instance_key
                    for item in action.requirement_instances
                ],
            }
            if action is not None
            else None
        ),
    }


def _expect_equal(label: str, expected: Any, actual: Any) -> None:
    if expected != actual:
        path, expected_value, actual_value = _first_difference(
            expected,
            actual,
        )
        raise GoldenMismatch(
            f"{label} mismatch at {path}: "
            f"expected {expected_value!r}, got {actual_value!r}"
        )


def _first_difference(
    expected: Any,
    actual: Any,
    path: str = "$",
) -> tuple[str, Any, Any]:
    """Return one focused difference instead of dumping an entire fixture."""

    if isinstance(expected, dict) and isinstance(actual, dict):
        for key in sorted(set(expected) | set(actual)):
            child_path = f"{path}.{key}"
            if key not in expected:
                return child_path, "<absent>", actual[key]
            if key not in actual:
                return child_path, expected[key], "<absent>"
            if expected[key] != actual[key]:
                return _first_difference(
                    expected[key],
                    actual[key],
                    child_path,
                )
    if isinstance(expected, list) and isinstance(actual, list):
        common_length = min(len(expected), len(actual))
        for index in range(common_length):
            if expected[index] != actual[index]:
                return _first_difference(
                    expected[index],
                    actual[index],
                    f"{path}[{index}]",
                )
        if len(expected) != len(actual):
            return f"{path}.length", len(expected), len(actual)
    return path, expected, actual


def _resolve_inside(root: Path, relative: str) -> Path:
    relative_path = Path(relative)
    if relative_path.is_absolute():
        raise GoldenMismatch(f"golden path must be relative: {relative}")
    resolved_root = root.resolve()
    candidate = (resolved_root / relative_path).resolve()
    try:
        candidate.relative_to(resolved_root)
    except ValueError as error:
        raise GoldenMismatch(
            f"golden path escapes suite root: {relative}"
        ) from error
    return candidate


def _contains_float(value: Any) -> bool:
    if isinstance(value, float):
        return True
    if isinstance(value, list):
        return any(_contains_float(item) for item in value)
    if isinstance(value, dict):
        return any(_contains_float(item) for item in value.values())
    return False
