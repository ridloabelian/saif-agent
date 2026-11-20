# Security Policy

## Reporting a Vulnerability

Please do **not** report security vulnerabilities through public GitHub issues, pull requests, or discussions.

Instead, report vulnerabilities privately through GitHub Security Advisories:

- Open a private report at [`/security/advisories/new`](https://github.com/ridloabelian/saif-agent/security/advisories/new).

Please include:
- A clear description of the issue and affected crate/command
- Steps to reproduce or a minimal proof-of-concept
- Potential impact (e.g., credential leakage, secret boundary bypass, prompt/skill injection, path traversal)

## Built-in Security Invariants

`saif-agent` enforces strict defense-in-depth invariants verified in conformance tests:
- **Fail-Closed `SecretScope`**: Profile-scoped credentials cannot silently fall back across profile boundaries or leak across gateway multiplex instances.
- **Structured Output Redaction**: `redact_sensitive_text` and `SecretRedactor` scrub API keys, bearer tokens, and `.env` secrets before tool previews, browser logs, or outbound errors are emitted.
- **Home Boundary Guards**: Profile paths validate `FileAtHomeBoundary` and honor tombstones to prevent path traversal or resurrection of deleted profiles.
- **Skill Threat Scanning**: `scan_for_threats` inspects imported `SKILL.md` skills for prompt-injection and suspicious shell payloads.
