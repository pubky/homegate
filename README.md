# Homegate

A backend service to gatekeep [Pubky Homeserver](https://github.com/pubky/pubky-core/) signups.

## SMS Verification

We use [Prelude](https://docs.prelude.so/) for SMS verification. Keep in mind that each phone number:
- Has a maximum of 10 verifications.
- Has a single pending verification at a time. Multiple `send_code` calls reuse the existing session.
