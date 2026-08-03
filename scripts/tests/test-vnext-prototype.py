#!/usr/bin/env python3
from __future__ import annotations

from copy import deepcopy
import json
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import unittest

import yaml

KIT_ROOT = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(KIT_ROOT / "prototype" / "vnext"))

from agentic_vnext import (  # noqa: E402
    Application,
    DerivedCache,
    FileProjectStore,
    GoldenMismatch,
    GitRepositoryAdapter,
    InMemoryProjectStore,
    SchemaValidationError,
    build_framework_lock,
    default_schema_registry,
    load_framework_lock,
    load_project,
    load_rule_source,
    evaluate_clean_clone,
    verify_golden_suite,
)
from agentic_vnext.model import canonical_digest  # noqa: E402


FIXTURE = KIT_ROOT / "prototype" / "vnext" / "fixtures" / "db-sqs"
CLI_FIXTURE = KIT_ROOT / "prototype" / "vnext" / "fixtures" / "cli-project"
GOLDEN = KIT_ROOT / "prototype" / "vnext" / "golden" / "v1"
CHANGE_ID = "change.place-order"


class VNextPrototypeTest(unittest.TestCase):
    def setUp(self) -> None:
        self.store = InMemoryProjectStore(load_project(FIXTURE / "project.yaml"))
        self.rule_source = load_rule_source(FIXTURE / "rules.yaml")
        self.framework_lock = load_framework_lock(
            FIXTURE / "framework-lock.yaml"
        )
        self.app = Application(
            self.store,
            self.rule_source,
            self.framework_lock,
        )

    def submit(self, response, payload, output_refs=()):
        action = response.decision.action
        context = response.context
        self.assertIsNotNone(action)
        self.assertIsNotNone(context)
        return self.app.submit(
            change_id=CHANGE_ID,
            action_id=action.id,
            context_digest=context.digest,
            role=action.role,
            result_schema=action.expected_result_schema,
            payload=payload,
            output_refs=tuple(output_refs),
        )

    def satisfied_outcomes(self, response):
        return [
            {
                "instance_key": instance.instance_key,
                "definition_digest": instance.definition_digest,
                "status": "satisfied",
                "summary": f"{instance.requirement_id}をfixtureで確認した",
                "basis_refs": sorted(
                    response.context.instance_source_digests[
                        instance.instance_key
                    ]
                ),
            }
            for instance in response.decision.action.requirement_instances
        ]

    def confirmed_candidate_review(self, candidate):
        return {
            "fingerprint": candidate["fingerprint"],
            "status": "confirmed",
            "reason": "検出根拠を確認し、適用対象と判断した",
            "basis_refs": list(candidate["evidence_refs"]),
        }

    def contract(self, contract_id):
        return next(
            contract
            for contract in self.store.snapshot(CHANGE_ID).contracts
            if contract["id"] == contract_id
        )

    def run_git(self, root, *arguments):
        return subprocess.run(
            ["git", "-C", str(root), *arguments],
            check=True,
            capture_output=True,
            text=True,
        ).stdout.strip()

    def write_yaml(self, path, value):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(
            yaml.safe_dump(value, allow_unicode=True, sort_keys=False),
            encoding="utf-8",
        )

    def review_signals(self):
        response = self.app.next(CHANGE_ID)
        self.assertEqual("needs-analysis", response.decision.state)
        self.assertEqual(
            ["risk-signals-reviewed"],
            [
                item.requirement_id
                for item in response.decision.action.requirement_instances
            ],
        )
        reviews = [
            self.confirmed_candidate_review(candidate)
            for candidate in response.context.payload["signal_candidates"]
        ]
        self.assertEqual(3, len(reviews))
        return self.submit(
            response,
            {
                "reviewed_candidates": reviews,
                "outcomes": self.satisfied_outcomes(response),
            },
        )

    def advance_to_contract_decision(self):
        response = self.review_signals()
        self.assertEqual("needs-analysis", response.decision.state)

        operation_instances = [
            item
            for item in response.decision.requirement_instances
            if item.instance_key
            == "operation-boundaries-confirmed|operation.place-order"
        ]
        self.assertEqual(1, len(operation_instances))
        self.assertEqual(2, len(operation_instances[0].selected_by))
        manifests = response.context.instance_source_digests
        affected_key = "affected-data-confirmed|data.orders"
        boundary_key = "operation-boundaries-confirmed|operation.place-order"
        platform_key = "platform-behavior-verified|integration.order-events"
        self.assertNotIn("contract.order-lifecycle", manifests[affected_key])
        self.assertIn("contract.order-lifecycle", manifests[boundary_key])
        self.assertNotIn("contract.customer-profile", manifests[boundary_key])
        self.assertNotIn("contract.order-lifecycle", manifests[platform_key])

        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
        )
        recorded_outcomes = {
            outcome["instance_key"]: outcome
            for result in self.store.snapshot(CHANGE_ID).results
            for outcome in result["payload"].get("outcomes", [])
        }
        self.assertNotIn(
            "contract.order-lifecycle",
            recorded_outcomes[affected_key]["freshness_refs"],
        )
        self.assertIn(
            "contract.order-lifecycle",
            recorded_outcomes[boundary_key]["freshness_refs"],
        )
        self.assertNotIn(
            "contract.order-lifecycle",
            recorded_outcomes[platform_key]["freshness_refs"],
        )
        self.assertEqual("needs-analysis", response.decision.state)
        self.assertEqual(
            {
                "data-contracts-ready",
                "distributed-effect-contracts-ready",
            },
            {
                item.requirement_id
                for item in response.decision.action.requirement_instances
            },
        )
        response = self.submit(
            response,
            {
                "outcomes": [],
                "decision_requests": [
                    {
                        "id": "decision-request.submission-result",
                        "question": (
                            "DB保存後にSQS送信が失敗した場合、受付結果を何と定義するか"
                        ),
                        "known_fact_refs": [
                            "contract.order-lifecycle",
                            "decision.order-model",
                        ],
                    }
                ],
            },
        )
        self.assertEqual("needs-human-decision", response.decision.state)
        return response

    def record_decision(self, response):
        response = self.submit(
            response,
            {
                "answers": [
                    {
                        "request_id": "decision-request.submission-result",
                        "selection": "DB保存を受付成功とし、SQS送信は再試行する",
                    }
                ]
            },
        )
        self.assertEqual("needs-decision-recording", response.decision.state)

        self.store.upsert_decision(
            {
                "schema_version": "1",
                "id": "decision.submission-result",
                "change_id": CHANGE_ID,
                "status": "accepted",
                "title": "DB保存完了を注文受付成功とする",
                "resolves": ["decision-request.submission-result"],
            }
        )
        self.store.upsert_contract(
            {
                "schema_version": "1",
                "id": "contract.order-lifecycle",
                "change_id": CHANGE_ID,
                "applies_to": [
                    "operation.place-order",
                    "data.orders",
                    "integration.order-events",
                ],
                "clauses": [
                    {
                        "id": "orders-source-of-truth",
                        "text": "受理済み注文の正本はordersテーブルとする",
                        "authority_ref": "decision.order-model",
                    },
                    {
                        "id": "order-created-once",
                        "text": "同じ受付IDから注文を複数作成しない",
                        "authority_ref": "decision.order-model",
                    },
                    {
                        "id": "submission-result",
                        "text": "DB保存完了を受付成功とし、SQS送信失敗は再試行する",
                        "authority_ref": "decision.submission-result",
                    },
                ],
            }
        )
        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
            output_refs=(
                "contract.order-lifecycle",
                "decision.submission-result",
            ),
        )
        # Contract更新により、そのContractを入力に含めた以前の分析はstaleになる。
        # affected-dataとplatformは同じActionのResultだがContractを読んでいないため、
        # operation-boundariesだけが再要求される。
        self.assertEqual("needs-analysis", response.decision.state)
        self.assertEqual(
            ["operation-boundaries-confirmed|operation.place-order"],
            [
                item.instance_key
                for item in response.decision.action.requirement_instances
            ],
        )
        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
        )
        self.assertEqual("needs-pre-build-challenge", response.decision.state)
        return response

    def advance_to_ready_to_build(self):
        response = self.record_decision(self.advance_to_contract_decision())
        challenge_instances = [
            item
            for item in response.decision.requirement_instances
            if item.instance_key == "design-challenged|operation.place-order"
        ]
        self.assertEqual(1, len(challenge_instances))
        self.assertEqual(2, len(challenge_instances[0].selected_by))
        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
        )
        self.assertEqual("ready-to-build", response.decision.state)
        return response

    def test_same_input_produces_same_decision(self):
        first = self.app.next(CHANGE_ID)
        second = self.app.next(CHANGE_ID)
        self.assertEqual(first.decision.as_dict(), second.decision.as_dict())
        self.assertEqual(first.context.digest, second.context.digest)

    def test_framework_lock_matches_runtime_and_rule_set(self):
        expected = build_framework_lock(
            self.rule_source,
            self.app.rule_index,
        )
        self.assertEqual(expected, self.framework_lock)
        self.assertTrue(self.app.framework_lock.digest.startswith("sha256:"))
        self.assertEqual(
            default_schema_registry().digest,
            self.framework_lock["schema_bundle"]["digest"],
        )

    def test_framework_lock_v2_preserves_strict_runtime_validation(self):
        from agentic_vnext.framework_lock import validate_framework_lock

        signed = deepcopy(self.framework_lock)
        signed["schema_version"] = "2"
        signed["release_artifact"] = {
            "artifact_digest": "sha256:" + "a" * 64,
            "source_id": "offline:test-fixture",
            "signer_key_id": "test.framework.release",
        }
        validated = validate_framework_lock(
            signed,
            self.rule_source,
            self.app.rule_index,
        )
        self.assertEqual("2", validated.manifest["schema_version"])

        signed["protocols"]["kernel"] = "unexpected"
        with self.assertRaisesRegex(ValueError, "protocols.kernel"):
            validate_framework_lock(
                signed,
                self.rule_source,
                self.app.rule_index,
            )

    def test_language_neutral_golden_suite_matches_runtime(self):
        verify_golden_suite(GOLDEN)

    def test_golden_verifier_detects_reviewed_output_change(self):
        from agentic_vnext.golden import load_golden, verify_application_case

        case = load_golden(GOLDEN / "application-initial.json")
        case["expected"]["decision"]["state"] = "ready-to-merge"
        with self.assertRaisesRegex(
            GoldenMismatch,
            "db-sqs.initial output mismatch",
        ):
            verify_application_case(case)

    def test_lifecycle_golden_detects_checkpoint_change(self):
        from agentic_vnext.golden import (
            load_golden,
            verify_application_scenario,
        )

        scenario = load_golden(GOLDEN / "application-lifecycle.json")
        scenario["steps"][-1]["expected"]["state"] = "needs-evidence"
        with self.assertRaisesRegex(
            GoldenMismatch,
            r"db-sqs.full-lifecycle step 11 mismatch at \$\.state",
        ):
            verify_application_scenario(scenario, GOLDEN)

    def test_framework_lock_rejects_changed_rule_source(self):
        changed_rules = deepcopy(self.rule_source)
        changed_rules["requirements"][0]["context"].append("affected-code")
        with self.assertRaisesRegex(
            ValueError,
            "rule_set.source_digest",
        ):
            Application(
                self.store,
                changed_rules,
                self.framework_lock,
            )

    def test_rule_compile_rejects_unsupported_result_schema(self):
        changed_rules = deepcopy(self.rule_source)
        changed_rules["requirements"][0]["result_schema"] = "result.unknown"
        with self.assertRaisesRegex(
            ValueError,
            "refers to unsupported Result schema: result.unknown",
        ):
            Application(
                self.store,
                changed_rules,
                self.framework_lock,
            )

    def test_rule_compile_rejects_role_not_allowed_by_result_schema(self):
        changed_rules = deepcopy(self.rule_source)
        changed_rules["requirements"][0]["role"] = "Challenger"
        with self.assertRaisesRegex(
            ValueError,
            "cannot use role Challenger with result.risk-signal-review",
        ):
            Application(
                self.store,
                changed_rules,
                self.framework_lock,
            )

    def test_framework_lock_rejects_protocol_version_mismatch(self):
        changed_lock = deepcopy(self.framework_lock)
        changed_lock["protocols"]["kernel"] = "999"
        with self.assertRaisesRegex(
            ValueError,
            "protocols.kernel",
        ):
            Application(
                self.store,
                self.rule_source,
                changed_lock,
            )

    def test_filesystem_store_reproduces_state_after_restart(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            first_store = FileProjectStore.initialize(temporary_root, project)
            self.assertEqual(
                InMemoryProjectStore(project).snapshot(CHANGE_ID).digest,
                first_store.snapshot(CHANGE_ID).digest,
            )
            first_app = Application(
                first_store,
                self.rule_source,
                self.framework_lock,
            )
            response = first_app.next(CHANGE_ID)
            reviews = [
                self.confirmed_candidate_review(candidate)
                for candidate in response.context.payload["signal_candidates"]
            ]
            response = first_app.submit(
                change_id=CHANGE_ID,
                action_id=response.decision.action.id,
                context_digest=response.context.digest,
                role=response.decision.action.role,
                result_schema=response.decision.action.expected_result_schema,
                payload={
                    "reviewed_candidates": reviews,
                    "outcomes": self.satisfied_outcomes(response),
                },
            )

            restarted_store = FileProjectStore(
                temporary_root,
                project["repository"],
            )
            restarted_app = Application(
                restarted_store,
                self.rule_source,
                self.framework_lock,
            )
            restarted = restarted_app.next(CHANGE_ID)
            self.assertEqual(
                response.decision.as_dict(),
                restarted.decision.as_dict(),
            )
            self.assertEqual(1, len(restarted_store.snapshot(CHANGE_ID).results))

    def test_filesystem_store_initialization_never_overwrites_records(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            first = FileProjectStore.initialize(temporary_root, project)
            original_digest = first.snapshot(CHANGE_ID).digest
            with self.assertRaisesRegex(ValueError, "would overwrite"):
                FileProjectStore.initialize(temporary_root, project)
            restarted = FileProjectStore(
                temporary_root,
                project["repository"],
            )
            self.assertEqual(original_digest, restarted.snapshot(CHANGE_ID).digest)

    def test_markdown_records_match_yaml_model_and_preserve_prose(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(
                temporary_root,
                project,
                document_format="markdown",
            )
            initial = store.snapshot(CHANGE_ID)
            self.assertEqual(
                InMemoryProjectStore(project).snapshot(CHANGE_ID).digest,
                initial.digest,
            )
            contract_path = (
                Path(temporary_root)
                / ".agentic"
                / "contracts"
                / "contract.order-lifecycle.md"
            )
            original_text = contract_path.read_text(encoding="utf-8")
            human_prose = (
                "## Operational rationale\n\n"
                "The orders table remains authoritative while event delivery "
                "is retried.\n\n"
            )
            contract_path.write_text(
                original_text.replace(
                    "```agentic-contract\n",
                    human_prose + "```agentic-contract\n",
                ),
                encoding="utf-8",
            )
            # Narrative prose is readable documentation, not a machine Contract.
            self.assertEqual(initial.digest, store.snapshot(CHANGE_ID).digest)

            contract = next(
                deepcopy(value)
                for value in store.snapshot(CHANGE_ID).contracts
                if value["id"] == "contract.order-lifecycle"
            )
            contract["clauses"].append(
                {
                    "id": "markdown-update",
                    "text": "構造化blockだけを安全に更新する",
                }
            )
            store.upsert_contract(contract)
            updated_text = contract_path.read_text(encoding="utf-8")
            self.assertIn(human_prose.strip(), updated_text)
            self.assertIn("markdown-update", updated_text)
            self.assertNotEqual(initial.digest, store.snapshot(CHANGE_ID).digest)

    def test_markdown_update_rejects_invalid_contract_without_changing_file(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(
                temporary_root,
                project,
                document_format="markdown",
            )
            path = (
                Path(temporary_root)
                / ".agentic"
                / "contracts"
                / "contract.order-lifecycle.md"
            )
            before = path.read_text(encoding="utf-8")
            invalid = deepcopy(self.contract("contract.order-lifecycle"))
            invalid["unexpected_field"] = True

            with self.assertRaisesRegex(
                SchemaValidationError,
                r"\$\.unexpected_field: unexpected field",
            ):
                store.upsert_contract(invalid)

            self.assertEqual(before, path.read_text(encoding="utf-8"))

    def test_snapshot_rejects_invalid_record_loaded_from_markdown(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(
                temporary_root,
                project,
                document_format="markdown",
            )
            path = (
                Path(temporary_root)
                / ".agentic"
                / "contracts"
                / "contract.order-lifecycle.md"
            )
            text = path.read_text(encoding="utf-8")
            path.write_text(
                text.replace(
                    "id: contract.order-lifecycle\n",
                    "id: contract.order-lifecycle\nunexpected_field: true\n",
                    1,
                ),
                encoding="utf-8",
            )

            with self.assertRaisesRegex(
                SchemaValidationError,
                r"\$\.unexpected_field: unexpected field",
            ):
                store.snapshot(CHANGE_ID)

    def test_initialization_rejects_invalid_project_before_writing_files(self):
        project = load_project(FIXTURE / "project.yaml")
        project["changes"][0].pop("intent")
        with tempfile.TemporaryDirectory() as temporary_root:
            with self.assertRaisesRegex(
                SchemaValidationError,
                "missing required field 'intent'",
            ):
                FileProjectStore.initialize(temporary_root, project)
            self.assertEqual([], list(Path(temporary_root).rglob("*")))

    def test_markdown_store_completes_human_decision_without_rewriting_prose(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            self.store = FileProjectStore.initialize(
                temporary_root,
                project,
                document_format="markdown",
            )
            self.app = Application(
                self.store,
                self.rule_source,
                self.framework_lock,
            )
            contract_path = (
                Path(temporary_root)
                / ".agentic"
                / "contracts"
                / "contract.order-lifecycle.md"
            )
            text = contract_path.read_text(encoding="utf-8")
            contract_path.write_text(
                text.replace(
                    "```agentic-contract\n",
                    "Project-owned explanation remains here.\n\n"
                    "```agentic-contract\n",
                ),
                encoding="utf-8",
            )

            self.advance_to_ready_to_build()
            self.assertIn(
                "Project-owned explanation remains here.",
                contract_path.read_text(encoding="utf-8"),
            )
            self.assertTrue(
                (
                    Path(temporary_root)
                    / ".agentic"
                    / "decisions"
                    / "decision.submission-result.md"
                ).is_file()
            )

    def test_markdown_record_rejects_ambiguous_structured_blocks(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(
                temporary_root,
                project,
                document_format="markdown",
            )
            contract_path = (
                Path(temporary_root)
                / ".agentic"
                / "contracts"
                / "contract.order-lifecycle.md"
            )
            with contract_path.open("a", encoding="utf-8") as stream:
                stream.write(
                    "\n```agentic-contract\n"
                    "id: contract.ambiguous\n"
                    "```\n"
                )
            with self.assertRaisesRegex(ValueError, "exactly one"):
                store.snapshot(CHANGE_ID)

    def test_filesystem_store_persists_contract_and_decision_updates(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            self.store = FileProjectStore.initialize(temporary_root, project)
            self.app = Application(
                self.store,
                self.rule_source,
                self.framework_lock,
            )
            before_restart = self.advance_to_ready_to_build()

            restarted_store = FileProjectStore(
                temporary_root,
                project["repository"],
            )
            restarted_app = Application(
                restarted_store,
                self.rule_source,
                self.framework_lock,
            )
            after_restart = restarted_app.next(CHANGE_ID)
            self.assertEqual("ready-to-build", before_restart.decision.state)
            self.assertEqual("ready-to-build", after_restart.decision.state)
            self.assertTrue(
                any(
                    decision["id"] == "decision.submission-result"
                    for decision in restarted_store.snapshot(CHANGE_ID).decisions
                )
            )

    def test_filesystem_store_rejects_duplicate_result_file(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            first = FileProjectStore.initialize(temporary_root, project)
            second = FileProjectStore(
                temporary_root,
                project["repository"],
            )
            result = {
                "schema_version": "1",
                "id": "result.duplicate-test",
                "change_id": CHANGE_ID,
                "action_id": "action.duplicate-test",
                "role": "Builder",
                "result_schema": "result.build",
                "context_digest": "sha256:" + ("a" * 64),
                "input_refs": {},
                "output_refs": [],
                "freshness_refs": {},
                "payload": {"summary": "duplicate write test"},
            }
            first.append_result(result)
            with self.assertRaisesRegex(ValueError, "record already exists"):
                second.append_result(result)

    def test_filesystem_store_rejects_unsafe_source_root(self):
        with tempfile.TemporaryDirectory() as temporary_root:
            with self.assertRaisesRegex(ValueError, "escapes repository"):
                FileProjectStore(
                    temporary_root,
                    {},
                    contract_root="../contracts",
                )
            with self.assertRaisesRegex(ValueError, "generated/local"):
                FileProjectStore(
                    temporary_root,
                    {},
                    contract_root=".agentic/cache/contracts",
                )

    def test_derived_cache_can_be_deleted_and_regenerated(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(temporary_root, project)
            cache = DerivedCache(temporary_root)
            app = Application(
                store,
                self.rule_source,
                self.framework_lock,
                cache=cache,
            )
            first = app.next(CHANGE_ID)
            snapshot_digest = store.snapshot(CHANGE_ID).digest
            cache_root = Path(temporary_root) / ".agentic" / "cache"
            self.assertTrue(
                (cache_root / "manifests" / f"{CHANGE_ID}.json").is_file()
            )
            self.assertTrue(
                (cache_root / "state" / f"{CHANGE_ID}.json").is_file()
            )

            shutil.rmtree(cache_root)
            second = app.next(CHANGE_ID)
            self.assertEqual(first.decision.as_dict(), second.decision.as_dict())
            self.assertEqual(first.context.digest, second.context.digest)
            self.assertEqual(snapshot_digest, store.snapshot(CHANGE_ID).digest)
            self.assertTrue(
                (cache_root / "manifests" / f"{CHANGE_ID}.json").is_file()
            )

    def test_corrupt_derived_cache_is_overwritten_from_sources(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_root:
            store = FileProjectStore.initialize(temporary_root, project)
            app = Application(
                store,
                self.rule_source,
                self.framework_lock,
                cache=DerivedCache(temporary_root),
            )
            expected = app.next(CHANGE_ID)
            state_path = (
                Path(temporary_root)
                / ".agentic"
                / "cache"
                / "state"
                / f"{CHANGE_ID}.json"
            )
            state_path.write_text("{broken", encoding="utf-8")

            actual = app.next(CHANGE_ID)
            with state_path.open(encoding="utf-8") as stream:
                cached_state = json.load(stream)
            self.assertEqual(expected.decision.as_dict(), actual.decision.as_dict())
            normalized_decision = json.loads(
                json.dumps(actual.decision.as_dict())
            )
            self.assertEqual(normalized_decision, cached_state)

    def test_cache_write_failure_does_not_block_kernel_decision(self):
        class FailingCache:
            def write_evaluation(self, *args, **kwargs):
                raise OSError("simulated disk failure")

        app = Application(
            self.store,
            self.rule_source,
            self.framework_lock,
            cache=FailingCache(),
        )
        response = app.next(CHANGE_ID)
        self.assertEqual("needs-analysis", response.decision.state)
        self.assertIn("simulated disk failure", response.cache_diagnostics[0])

    def test_missing_detection_coverage_blocks_workflow(self):
        project = load_project(FIXTURE / "project.yaml")
        del project["repository"]["coverage"]
        app = Application(
            InMemoryProjectStore(project),
            self.rule_source,
            self.framework_lock,
        )

        response = app.next(CHANGE_ID)

        self.assertEqual("blocked-detection", response.decision.state)
        self.assertIsNone(response.decision.action)
        self.assertIn(
            "coverage-not-reported",
            response.decision.diagnostics[0],
        )

    def test_unknown_repository_fact_kind_is_rejected(self):
        project = load_project(FIXTURE / "project.yaml")
        project["repository"]["facts"].append(
            {
                "kind": "cache_read",
                "evidence_refs": ["code.place-order-handler"],
            }
        )
        app = Application(
            InMemoryProjectStore(project),
            self.rule_source,
            self.framework_lock,
        )

        with self.assertRaisesRegex(
            ValueError,
            "unsupported kind: cache_read",
        ):
            app.next(CHANGE_ID)

    def test_git_adapter_marks_unscanned_declared_artifact_incomplete(self):
        with tempfile.TemporaryDirectory() as temporary:
            source_root = Path(temporary) / "source"
            shutil.copytree(CLI_FIXTURE, source_root)
            self.run_git(source_root, "init")
            self.run_git(source_root, "config", "user.email", "ci@example.test")
            self.run_git(source_root, "config", "user.name", "CI Fixture")
            self.run_git(source_root, "add", ".")
            self.run_git(source_root, "commit", "-m", "initial observation")
            manifest_path = (
                source_root / ".agentic" / "repository-observation.yaml"
            )
            manifest = yaml.safe_load(manifest_path.read_text(encoding="utf-8"))
            manifest["coverage"]["analyzed_refs"].remove(
                "code.order-events-publisher"
            )
            self.write_yaml(manifest_path, manifest)

            repository = GitRepositoryAdapter(
                source_root,
                ".agentic/repository-observation.yaml",
                require_clean=False,
            ).observe()

            self.assertEqual("incomplete", repository["coverage"]["status"])
            self.assertEqual(
                "unscanned-artifact",
                repository["coverage"]["gaps"][0]["kind"],
            )
            self.assertEqual(
                "code.order-events-publisher",
                repository["coverage"]["gaps"][0]["ref"],
            )

    def test_clean_git_clone_reproduces_ready_to_merge(self):
        project = load_project(FIXTURE / "project.yaml")
        with tempfile.TemporaryDirectory() as temporary_parent:
            source_root = Path(temporary_parent) / "source"
            clone_root = Path(temporary_parent) / "clone"
            source_root.mkdir()
            (source_root / "src").mkdir()
            (source_root / "src" / "place_order.py").write_text(
                "def place_order():\n    return 'stored'\n",
                encoding="utf-8",
            )
            (source_root / "src" / "publish_order.py").write_text(
                "def publish_order():\n    return 'queued'\n",
                encoding="utf-8",
            )
            observation_manifest = {
                "schema_version": "2",
                "phase": "pre-build",
                "artifacts": [
                    {
                        "ref": "code.place-order-handler",
                        "path": "src/place_order.py",
                        "applies_to": [
                            "operation.place-order",
                            "data.orders",
                        ],
                    },
                    {
                        "ref": "code.order-events-publisher",
                        "path": "src/publish_order.py",
                        "applies_to": [
                            "operation.place-order",
                            "integration.order-events",
                        ],
                    },
                ],
                "coverage": {
                    "scope": "declared-artifacts",
                    "analyzed_refs": [
                        "code.place-order-handler",
                        "code.order-events-publisher",
                    ],
                    "gaps": [],
                },
                "facts": deepcopy(project["repository"]["facts"]),
            }
            self.write_yaml(
                source_root / ".agentic" / "repository-observation.yaml",
                observation_manifest,
            )
            self.write_yaml(
                source_root / ".agentic" / "config.yaml",
                {
                    "schema_version": "1",
                    "project_sources": {
                        "contracts": ".agentic/contracts",
                        "decisions": ".agentic/decisions",
                    },
                    "repository_observation": (
                        ".agentic/repository-observation.yaml"
                    ),
                },
            )
            shutil.copy2(
                FIXTURE / "framework-lock.yaml",
                source_root / ".agentic" / "framework.lock",
            )
            (source_root / ".gitignore").write_text(
                ".agentic/cache/\n",
                encoding="utf-8",
            )

            # Commit code and observation declarations first so the Git Adapter
            # can calculate the exact artifact identity used by all Result refs.
            self.run_git(source_root, "init")
            self.run_git(source_root, "config", "user.email", "ci@example.test")
            self.run_git(source_root, "config", "user.name", "CI Fixture")
            self.run_git(source_root, "add", ".")
            self.run_git(source_root, "commit", "-m", "initial observation")

            repository = GitRepositoryAdapter(
                source_root,
                ".agentic/repository-observation.yaml",
            ).observe()
            project["repository"] = repository
            self.store = FileProjectStore.initialize(source_root, project)
            self.app = Application(
                self.store,
                self.rule_source,
                self.framework_lock,
            )
            self.advance_to_ready_to_build()

            post_build_repository = deepcopy(repository)
            post_build_repository["phase"] = "post-build"
            self.store.update_repository(post_build_repository)
            observation_manifest["phase"] = "post-build"
            self.write_yaml(
                source_root / ".agentic" / "repository-observation.yaml",
                observation_manifest,
            )
            response = self.app.next(CHANGE_ID)
            self.assertEqual("needs-evidence", response.decision.state)
            response = self.submit(
                response,
                {"outcomes": self.satisfied_outcomes(response)},
            )
            response = self.submit(
                response,
                {"outcomes": self.satisfied_outcomes(response)},
            )
            self.assertEqual("ready-to-merge", response.decision.state)

            self.run_git(source_root, "add", ".")
            self.run_git(source_root, "commit", "-m", "record completed change")
            subprocess.run(
                ["git", "clone", "--quiet", str(source_root), str(clone_root)],
                check=True,
                capture_output=True,
                text=True,
            )

            evaluation = evaluate_clean_clone(
                clone_root,
                CHANGE_ID,
                self.rule_source,
            )
            self.assertTrue(evaluation.merge_allowed)
            self.assertEqual("ready-to-merge", evaluation.state)
            self.assertEqual(
                self.run_git(clone_root, "rev-parse", "HEAD"),
                evaluation.revision,
            )

            # CI must not certify a checkout that contains uncommitted inputs.
            (clone_root / "src" / "place_order.py").write_text(
                "def place_order():\n    return 'changed-but-uncommitted'\n",
                encoding="utf-8",
            )
            with self.assertRaisesRegex(ValueError, "working tree is not clean"):
                evaluate_clean_clone(
                    clone_root,
                    CHANGE_ID,
                    self.rule_source,
                )

    def test_explain_traces_candidate_rule_and_requirement(self):
        initial = self.app.explain(CHANGE_ID)
        self.assertEqual("needs-analysis", initial.state)
        self.assertEqual("review-risk-signals", initial.next_action["action"])
        self.assertEqual(
            {"unreviewed"},
            {candidate["disposition"] for candidate in initial.candidates},
        )
        self.assertIn("state: needs-analysis", initial.render_text())

        self.review_signals()
        report = self.app.explain(CHANGE_ID)
        boundary = next(
            requirement
            for requirement in report.requirements
            if requirement["instance_key"]
            == "operation-boundaries-confirmed|operation.place-order"
        )
        self.assertEqual(2, len(boundary["selected_by"]))
        self.assertEqual(
            {"confirmed"},
            {candidate["disposition"] for candidate in report.candidates},
        )

    def test_explain_reports_stale_source_per_outcome(self):
        self.advance_to_ready_to_build()
        contract = deepcopy(self.contract("contract.order-lifecycle"))
        contract["clauses"].append(
            {"id": "explain-stale", "text": "説明用のContract変更"}
        )
        self.store.upsert_contract(contract)

        report = self.app.explain(CHANGE_ID)
        boundary = next(
            requirement
            for requirement in report.requirements
            if requirement["requirement_id"]
            == "operation-boundaries-confirmed"
        )
        affected_data = next(
            requirement
            for requirement in report.requirements
            if requirement["requirement_id"] == "affected-data-confirmed"
        )
        self.assertTrue(
            any(
                "contract.order-lifecycle" in check["stale_refs"]
                for check in boundary["result_checks"]
            )
        )
        self.assertEqual("unsatisfied", boundary["status"])
        self.assertEqual("satisfied", affected_data["status"])

    def test_wrong_result_subtype_cannot_satisfy_requirement(self):
        response = self.review_signals()
        instance = response.decision.action.requirement_instances[0]
        instance_refs = response.context.instance_source_digests[
            instance.instance_key
        ]
        forged_result = {
            "schema_version": "1",
            "id": "result.wrong-subtype",
            "change_id": CHANGE_ID,
            "action_id": "action.wrong-subtype",
            "role": "Challenger",
            "result_schema": "result.challenge",
            "context_digest": response.context.digest,
            "input_refs": response.context.source_digests,
            "output_refs": [],
            "freshness_refs": response.context.source_digests,
            "payload": {
                "outcomes": [
                    {
                        "instance_key": instance.instance_key,
                        "definition_digest": instance.definition_digest,
                        "status": "satisfied",
                        "summary": "異なるResult種別による偽の充足結果",
                        "basis_refs": sorted(instance_refs),
                        "input_refs": instance_refs,
                        "freshness_refs": instance_refs,
                    }
                ]
            },
        }
        self.store.append_result(forged_result)

        decision = self.app.next(CHANGE_ID)
        self.assertIn(
            instance.instance_key,
            {
                item.instance_key
                for item in decision.decision.action.requirement_instances
            },
        )
        report = self.app.explain(CHANGE_ID)
        trace = next(
            item
            for item in report.requirements
            if item["instance_key"] == instance.instance_key
        )
        check = next(
            item
            for item in trace["result_checks"]
            if item["result_id"] == "result.wrong-subtype"
        )
        self.assertFalse(check["result_schema_matches"])
        self.assertFalse(check["role_matches"])
        self.assertFalse(check["accepted"])

    def test_explain_traces_human_authority_lifecycle(self):
        response = self.advance_to_contract_decision()
        report = self.app.explain(CHANGE_ID)
        self.assertEqual("open", report.authority[0]["status"])

        self.submit(
            response,
            {
                "answers": [
                    {
                        "request_id": "decision-request.submission-result",
                        "selection": "DB保存を受付成功とし、SQS送信は再試行する",
                    }
                ]
            },
        )
        report = self.app.explain(CHANGE_ID)
        self.assertEqual("answered-not-recorded", report.authority[0]["status"])
        self.assertEqual("needs-decision-recording", report.state)

    def test_risk_review_only_returns_unreviewed_candidate_delta(self):
        response = self.app.next(CHANGE_ID)
        candidates = response.context.payload["signal_candidates"]
        first_candidate = candidates[0]
        response = self.submit(
            response,
            {
                "reviewed_candidates": [
                    self.confirmed_candidate_review(first_candidate)
                ],
                "outcomes": self.satisfied_outcomes(response),
            },
        )

        remaining = response.context.payload["signal_candidates"]
        self.assertEqual(2, len(remaining))
        self.assertNotIn(
            first_candidate["fingerprint"],
            {candidate["fingerprint"] for candidate in remaining},
        )

    def test_changed_evidence_requires_only_new_candidate_review(self):
        self.advance_to_ready_to_build()
        repository = deepcopy(self.store.snapshot(CHANGE_ID).repository)
        for artifact in repository["artifacts"]:
            if artifact["ref"] == "code.place-order-handler":
                artifact["digest"] = "sha256:" + ("3" * 64)
        self.store.update_repository(repository)

        response = self.app.next(CHANGE_ID)
        self.assertEqual("needs-analysis", response.decision.state)
        candidates = response.context.payload["signal_candidates"]
        self.assertEqual(1, len(candidates))
        self.assertEqual("persistent-data-write", candidates[0]["signal"])

        response = self.submit(
            response,
            {
                "reviewed_candidates": [
                    self.confirmed_candidate_review(candidates[0])
                ],
                "outcomes": self.satisfied_outcomes(response),
            },
        )
        self.assertEqual("needs-analysis", response.decision.state)
        self.assertEqual(
            {
                "affected-data-confirmed",
                "operation-boundaries-confirmed",
            },
            {
                instance.requirement_id
                for instance in response.decision.action.requirement_instances
            },
        )

    def test_db_and_sqs_flow_reaches_ready_to_merge(self):
        self.advance_to_ready_to_build()
        repository = deepcopy(self.store.snapshot(CHANGE_ID).repository)
        repository["phase"] = "post-build"
        repository["revision"] = "fixture-r2"
        self.store.update_repository(repository)

        response = self.app.next(CHANGE_ID)
        self.assertEqual("needs-evidence", response.decision.state)
        self.assertEqual("Builder", response.decision.action.role)
        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
        )
        self.assertEqual("needs-post-build-challenge", response.decision.state)
        response = self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
        )
        self.assertEqual("ready-to-merge", response.decision.state)
        self.assertIsNone(response.context)

    def test_human_decision_promoted_to_shared_contract_is_reused_by_second_change(
        self,
    ):
        response = self.advance_to_contract_decision()
        response = self.submit(
            response,
            {
                "answers": [
                    {
                        "request_id": "decision-request.submission-result",
                        "selection": "DB保存を受付成功とし、イベント送信は再試行する",
                    }
                ]
            },
        )
        self.assertEqual("needs-decision-recording", response.decision.state)

        shared_decision = {
            "schema_version": "1",
            "id": "decision.shared-order-submission",
            "status": "accepted",
            "title": "DB保存完了を注文受付成功とする",
            "resolves": ["decision-request.submission-result"],
        }
        shared_contract = {
            "schema_version": "1",
            "id": "contract.shared-order-submission",
            "applies_to": [
                "operation.place-order",
                "data.orders",
                "integration.order-events",
            ],
            "clauses": [
                {
                    "id": "submission-result",
                    "text": (
                        "DB保存完了を受付成功とし、イベント送信失敗は再試行する"
                    ),
                    "authority_ref": "decision.shared-order-submission",
                }
            ],
        }
        self.store.upsert_decision(shared_decision)
        self.store.upsert_contract(shared_contract)
        self.submit(
            response,
            {"outcomes": self.satisfied_outcomes(response)},
            output_refs=(
                "contract.shared-order-submission",
                "decision.shared-order-submission",
            ),
        )

        second_change_id = "change.retry-order-events"
        first_snapshot = self.store.snapshot(CHANGE_ID)
        second_snapshot = self.store.snapshot(second_change_id)
        self.assertIn(
            "contract.order-lifecycle",
            {contract["id"] for contract in first_snapshot.contracts},
        )
        self.assertNotIn(
            "contract.order-lifecycle",
            {contract["id"] for contract in second_snapshot.contracts},
        )
        self.assertIn(
            "contract.shared-order-submission",
            {contract["id"] for contract in second_snapshot.contracts},
        )
        self.assertNotIn(
            "decision.order-model",
            {decision["id"] for decision in second_snapshot.decisions},
        )
        self.assertIn(
            "decision.shared-order-submission",
            {decision["id"] for decision in second_snapshot.decisions},
        )

        def submit_second(current, payload):
            action = current.decision.action
            context = current.context
            self.assertIsNotNone(action)
            self.assertIsNotNone(context)
            return self.app.submit(
                change_id=second_change_id,
                action_id=action.id,
                context_digest=context.digest,
                role=action.role,
                result_schema=action.expected_result_schema,
                payload=payload,
            )

        response = self.app.next(second_change_id)
        saw_shared_context = False
        for _ in range(8):
            action = response.decision.action
            self.assertIsNotNone(action)
            self.assertNotEqual("Human", action.role)
            if action.role == "Challenger":
                break
            if action.action == "review-risk-signals":
                response = submit_second(
                    response,
                    {
                        "reviewed_candidates": [
                            self.confirmed_candidate_review(candidate)
                            for candidate in response.context.payload[
                                "signal_candidates"
                            ]
                        ],
                        "outcomes": self.satisfied_outcomes(response),
                    },
                )
                continue

            if "contract.shared-order-submission" in response.context.source_refs:
                saw_shared_context = True
                self.assertIn(
                    "decision.shared-order-submission",
                    response.context.source_refs,
                )
                self.assertNotIn(
                    "contract.order-lifecycle",
                    response.context.source_refs,
                )
            response = submit_second(
                response,
                {"outcomes": self.satisfied_outcomes(response)},
            )
        else:
            self.fail("second Change did not reach Challenger")

        self.assertTrue(saw_shared_context)
        self.assertEqual("needs-pre-build-challenge", response.decision.state)
        self.assertEqual("Challenger", response.decision.action.role)

    def test_shared_contract_update_rejects_missing_and_stale_digest(self):
        initial = {
            "schema_version": "1",
            "id": "contract.shared-concurrency",
            "applies_to": ["data.orders"],
            "clauses": [
                {
                    "id": "concurrency-policy",
                    "text": "初期の並行更新規範",
                }
            ],
        }
        self.store.upsert_contract(initial)
        initial_digest = canonical_digest(initial)

        first_update = deepcopy(initial)
        first_update["clauses"][0]["text"] = "先に確定した並行更新規範"
        stale_update = deepcopy(initial)
        stale_update["clauses"][0]["text"] = "古い版を基にした上書き"

        with self.assertRaisesRegex(
            ValueError,
            "Shared Contract update requires expected digest",
        ):
            self.store.upsert_contract(first_update)

        self.store.upsert_contract(first_update, initial_digest)
        with self.assertRaisesRegex(ValueError, "stale Contract update"):
            self.store.upsert_contract(stale_update, initial_digest)

        current = next(
            contract
            for contract in self.store.snapshot(CHANGE_ID).contracts
            if contract["id"] == initial["id"]
        )
        self.assertEqual(
            "先に確定した並行更新規範",
            current["clauses"][0]["text"],
        )

    def test_contract_change_makes_dependent_results_stale(self):
        self.advance_to_ready_to_build()
        contract = deepcopy(self.contract("contract.order-lifecycle"))
        contract["clauses"].append(
            {
                "id": "retry-limit",
                "text": "SQS送信は最大5回まで再試行する",
            }
        )
        self.store.upsert_contract(contract)

        response = self.app.next(CHANGE_ID)
        self.assertEqual("needs-analysis", response.decision.state)
        statuses = {
            item.requirement_id: item.status
            for item in response.decision.requirement_instances
        }
        self.assertEqual("satisfied", statuses["affected-data-confirmed"])
        self.assertEqual("satisfied", statuses["platform-behavior-verified"])
        self.assertEqual("unsatisfied", statuses["operation-boundaries-confirmed"])
        self.assertEqual("unsatisfied", statuses["data-contracts-ready"])
        self.assertEqual(
            "unsatisfied",
            statuses["distributed-effect-contracts-ready"],
        )

    def test_unrelated_contract_change_does_not_stale_results(self):
        before = self.advance_to_ready_to_build()
        unrelated = next(
            deepcopy(contract)
            for contract in self.store.snapshot(CHANGE_ID).contracts
            if contract["id"] == "contract.customer-profile"
        )
        unrelated["clauses"][0]["text"] = "顧客メールアドレスの原文も別項目へ保持する"
        self.store.upsert_contract(unrelated)

        after = self.app.next(CHANGE_ID)
        self.assertEqual("ready-to-build", before.decision.state)
        self.assertEqual("ready-to-build", after.decision.state)

    def test_submit_rejects_wrong_context_digest(self):
        response = self.app.next(CHANGE_ID)
        with self.assertRaisesRegex(ValueError, "context digest"):
            self.app.submit(
                change_id=CHANGE_ID,
                action_id=response.decision.action.id,
                context_digest="sha256:wrong",
                role="Analyst",
                result_schema="result.risk-signal-review",
                payload={},
            )

    def test_submit_rejects_payload_that_does_not_match_result_schema(self):
        response = self.app.next(CHANGE_ID)
        before_count = len(self.store.snapshot(CHANGE_ID).results)
        with self.assertRaisesRegex(
            SchemaValidationError,
            "missing required field 'reviewed_candidates'",
        ):
            self.submit(
                response,
                {"outcomes": self.satisfied_outcomes(response)},
            )
        self.assertEqual(
            before_count,
            len(self.store.snapshot(CHANGE_ID).results),
        )

    def test_submit_rejects_status_only_outcome_without_explanation(self):
        response = self.app.next(CHANGE_ID)
        outcomes = self.satisfied_outcomes(response)
        outcomes[0].pop("summary")
        with self.assertRaisesRegex(
            SchemaValidationError,
            "missing required field 'summary'",
        ):
            self.submit(
                response,
                {
                    "reviewed_candidates": [
                        self.confirmed_candidate_review(candidate)
                        for candidate in response.context.payload[
                            "signal_candidates"
                        ]
                    ],
                    "outcomes": outcomes,
                },
            )

    def test_submit_rejects_outcome_basis_outside_issued_context(self):
        response = self.review_signals()
        outcomes = self.satisfied_outcomes(response)
        outcomes[0]["basis_refs"].append("code.not-in-issued-context")
        with self.assertRaisesRegex(
            ValueError,
            "outcome basis ref does not belong to issued action",
        ):
            self.submit(response, {"outcomes": outcomes})

    def test_store_rejects_result_payload_for_different_subtype(self):
        result = {
            "schema_version": "1",
            "id": "result.wrong-payload",
            "change_id": CHANGE_ID,
            "action_id": "action.wrong-payload",
            "role": "Human",
            "result_schema": "result.human-answer",
            "context_digest": "sha256:" + ("a" * 64),
            "input_refs": {},
            "output_refs": [],
            "freshness_refs": {},
            "payload": {"outcomes": []},
        }
        with self.assertRaisesRegex(
            SchemaValidationError,
            r"\$\.payload: missing required field 'answers'",
        ):
            self.store.append_result(result)

    def test_submit_rejects_candidate_not_in_delta(self):
        response = self.app.next(CHANGE_ID)
        with self.assertRaisesRegex(ValueError, "candidate was not offered"):
            self.submit(
                response,
                {
                    "reviewed_candidates": [
                        {
                            "fingerprint": "sha256:" + ("f" * 64),
                            "status": "confirmed",
                            "reason": "提示されていない候補を送信する異常系",
                            "basis_refs": list(
                                response.context.payload[
                                    "signal_candidates"
                                ][0]["evidence_refs"]
                            ),
                        }
                    ],
                    "outcomes": self.satisfied_outcomes(response),
                },
            )

    def test_submit_rejects_unreported_input_change(self):
        response = self.review_signals()
        contract = deepcopy(self.contract("contract.order-lifecycle"))
        contract["clauses"].append(
            {"id": "changed-during-action", "text": "Action発行後の入力変更"}
        )
        self.store.upsert_contract(contract)
        with self.assertRaisesRegex(ValueError, "issued context is stale"):
            self.submit(
                response,
                {"outcomes": self.satisfied_outcomes(response)},
            )


if __name__ == "__main__":
    unittest.main()
