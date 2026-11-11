# T05: Account discovery

Crate / folder: `core/protocols`

## Goal
Users type only their email address (screens S01 to S03).

## Spec
- Order: built-in provider table (Gmail, Outlook, iCloud, Fastmail), Mozilla-style autoconfig XML (`autoconfig.<domain>`, then `<domain>/.well-known/autoconfig/mail/config-v1.1.xml`), RFC 6186 SRV records, JMAP `/.well-known/jmap`, then ask for manual settings (S05).
- Probes run together; the first match wins. Each failure produces a plain-language reason the UI can show.
- Pure parsing (autoconfig XML, SRV ordering) is unit-tested with recorded fixtures.

## Done when
- Discovery succeeds for three test servers and fails cleanly for a wrong password and for a self-signed certificate, each with its own message.
- Nothing is sent to any server other than the person's own domain and the provider table's hosts.
