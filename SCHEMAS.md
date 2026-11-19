# ISDS schema inputs

Retrieved from the official production static schema endpoint on 2026-11-19. The public files report WSDL/XSD version 3.11. These hashes identify the protocol inputs used for the initial wire mappings; schemas are not downloaded during a build.

Base URL: `https://www.datovka.gov.cz/static/wsdl/v20/`

| File | SHA-256 |
| --- | --- |
| `dbTypes.xsd` | `547c3b2183e13436200c10902430b364da8be38b26e31e9c455bbf6f994bd8f1` |
| `db_access.wsdl` | `c655b2e1cbe6b4f81ac707f9fac555fe056d2e91a81de71e305e320de99569ea` |
| `db_search.wsdl` | `e4540467025adbdf097803e741dc604eaef24fdf26ce7d38e4f5f590b2f53670` |
| `dmBaseTypes.xsd` | `780ec788bf88042d888838d12f18da9ce643e5a3caa95e4b4517514bc7abc5ea` |
| `dm_info.wsdl` | `b82c99e6cfbd80bdbdb20825cfdfba0a47d0894c4369a7f6708115984db8b5ac` |
| `dm_operations.wsdl` | `80c3ed286c5c2775be8db64813c41ec54509b94d3839132192b03a65589bf905` |

The static test WSDLs must not be used to select runtime endpoints: at the time of retrieval they contain production `soap:address` values. Runtime URLs are selected explicitly by `Environment`.

Authoritative context:

- [Operating Rules](https://datovka.gov.cz/info/cs/80.html)
- [Developer notices](https://datovka.gov.cz/info/cs/2052.html)
- [Test environment](https://datovka.gov.cz/info/cs/95.html)
