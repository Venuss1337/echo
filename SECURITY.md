# Security policy

## Reporting a vulnerability

Please report vulnerabilities **privately** through GitHub:
**Security → Report a vulnerability** on this repository
([direct link](https://github.com/Venuss1337/echo/security/advisories/new)).
Do not open a public issue, discussion or pull request for a security
problem.

Include the ECHO version, Windows version, what an attacker controls (a
file, tags, artwork, a radio station or directory entry, …), the impact, and
the smallest reproduction you can. Do not access other people's data or
disrupt third-party radio servers while investigating.

ECHO is maintained by volunteers, so there is no guaranteed response time.
The maintainers aim to acknowledge reports within a week, will keep you
informed, fix confirmed issues with a regression test, publish a GitHub
security advisory and credit you unless you prefer otherwise.

## Supported versions

| Version | Supported |
| --- | --- |
| Latest 1.x release | Yes |
| Older releases | No; please update |

## Scope

In scope: anything that lets content ECHO handles (music files, tags,
artwork, playlists, radio streams, directory data, station URLs) or the
renderer read or write files, reach other network destinations, run code,
or corrupt or delete user data; and weaknesses in the release process.

ECHO's [security model](docs/SECURITY.md) describes the boundaries and what
is deliberately not protected, such as native code safety in the decoder.
