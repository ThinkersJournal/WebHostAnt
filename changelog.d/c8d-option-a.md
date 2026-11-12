kind: C8d: `ACME_CONTACT` opts in to a non-loopback `BIND` and `HTTP_BIND`
---
Row C8d, PM ruling option A. A `LocalTarget` app that already holds `ACME_CONTACT` may set a `BIND` or `HTTP_BIND` beyond loopback (a certificate authority must reach port 80); without it, or `allow_non_loopback`, the setting is still refused at apply. Order matters: `ACME_CONTACT` first. The default authority stays Let's Encrypt staging. Verified by the changed refusal test (refused without the contact, accepted after it, refused if the contact comes after). Not breaking. No network, no real CA.
