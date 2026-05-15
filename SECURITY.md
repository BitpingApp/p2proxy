# Security Policy

## Reporting a Vulnerability

If you believe you've found a security vulnerability in p2proxy, **please do not file a public GitHub issue**. Public issues are visible to anyone watching the repo, including the people who could exploit the bug before we patch it.

Instead, email **security@bitping.com** with:

- A description of the issue
- Steps to reproduce (a proof-of-concept is appreciated but not required)
- The version of p2proxy you tested against (`p2proxy --version`)
- Your assessment of the impact (e.g. remote code execution, denial of service, information disclosure)

We aim to acknowledge reports within 2 business days and to ship a fix or mitigation within 30 days for high-severity issues.

## Supported Versions

Only the latest minor version is supported with security patches. Older versions may receive fixes at our discretion if the issue is severe enough.

## Scope

In scope:

- The `p2proxy` binary and its libp2p / SOCKS5 implementations
- The `Config.yaml` parser
- Anything that crosses a trust boundary (network input, configuration input, the auth flow)

Out of scope:

- Vulnerabilities in third-party dependencies — please report those upstream first; we'll coordinate the bump on our side.
- Issues that require local root, physical access, or already-compromised credentials to exploit.
- Denial-of-service via traffic volume against an exposed listener (run p2proxy behind a firewall).

## Coordinated Disclosure

We follow standard coordinated-disclosure practice: fix first, public advisory after the patch is widely deployed. We'll credit reporters in the advisory unless you'd prefer to stay anonymous.
