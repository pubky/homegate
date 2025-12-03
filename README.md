# Homegate

A backend service to gatekeep [Pubky Homeserver](https://github.com/pubky/pubky-core/) signups.

## SMS Verification

We use [Prelude](https://docs.prelude.so/) for SMS verification. Keep in mind that for each phone number:

- There is one pending verification at a time. Multiple `send_code` calls reuse the existing session.
- Can complete verification at most 10 times.
