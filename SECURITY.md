# Security policy

## Supported versions

AgenticVisor is in beta. Only the latest release receives fixes.

| Version        | Supported |
|----------------|-----------|
| 0.1.0-beta.x   | Yes       |

## Reporting a vulnerability

Please **do not open a public issue** for security problems.

Report them privately through GitHub: open the repository's **Security** tab and
choose **Report a vulnerability** (private vulnerability reporting). Include:

- what you found and where (file, endpoint or feature);
- the steps to reproduce it;
- the impact you expect, and the version you tested.

You should get an answer within 7 days. If the report is confirmed, we will fix
it, publish a release, and credit you in the release notes if you want that.

## Scope

AgenticVisor runs a local server on `127.0.0.1` and reads your AI tools' data
from your own home directory. Reports about the following are especially
welcome:

- any way to read or change files outside the documented locations;
- any way to reach the local server from another machine;
- any leak of credentials or conversation content through the interface or the API.
