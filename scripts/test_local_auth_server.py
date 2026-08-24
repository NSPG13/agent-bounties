from __future__ import annotations

import functools
import http.client
import importlib.util
import json
import tempfile
import threading
import unittest
import urllib.parse
from pathlib import Path
from unittest import mock


MODULE_PATH = Path(__file__).with_name("serve-solarpunk-auth.py")
SPEC = importlib.util.spec_from_file_location("serve_solarpunk_auth", MODULE_PATH)
assert SPEC and SPEC.loader
auth = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(auth)


class LocalAuthTests(unittest.TestCase):
    def setUp(self) -> None:
        auth.OAUTH_STATES.clear()
        auth.WALLET_CHALLENGES.clear()
        auth.PUBLIC_EVIDENCE_CACHE.update({"loaded_at": 0.0, "payload": None})
        self.env = {
            "AUTH_ORIGIN": "http://127.0.0.1:4173",
            "AUTH_SESSION_SECRET": "s" * 48,
            "GOOGLE_OAUTH_CLIENT_ID": "google-client.apps.googleusercontent.com",
            "GOOGLE_OAUTH_CLIENT_SECRET": "google-secret",
            "GOOGLE_OAUTH_REDIRECT_URI": "http://127.0.0.1:4173/auth/callback/google",
            "GITHUB_OAUTH_CLIENT_ID": "github-client",
            "GITHUB_OAUTH_CLIENT_SECRET": "github-secret",
            "GITHUB_OAUTH_REDIRECT_URI": "http://127.0.0.1:4173/auth/callback/github",
            "MICROSOFT_OAUTH_CLIENT_ID": "microsoft-client",
            "MICROSOFT_OAUTH_CLIENT_SECRET": "microsoft-secret",
            "MICROSOFT_OAUTH_TENANT": "common",
            "MICROSOFT_OAUTH_REDIRECT_URI": "http://localhost:4173/auth/callback/microsoft",
            "AMAZON_OAUTH_CLIENT_ID": "amazon-client",
            "AMAZON_OAUTH_CLIENT_SECRET": "amazon-secret",
            "AMAZON_OAUTH_REDIRECT_URI": "http://127.0.0.1:4173/auth/callback/amazon",
        }

    def test_signed_session_rejects_tampering_and_expiry(self) -> None:
        token = auth.sign_session(
            {"provider": "google", "sub": "123", "name": "Test", "email": "test@example.com"},
            self.env["AUTH_SESSION_SECRET"],
            now=100,
        )
        self.assertEqual(auth.verify_session(token, self.env["AUTH_SESSION_SECRET"], now=101)["sub"], "123")
        self.assertIsNone(auth.verify_session(token + "x", self.env["AUTH_SESSION_SECRET"], now=101))
        self.assertIsNone(auth.verify_session(token, self.env["AUTH_SESSION_SECRET"], now=100 + auth.SESSION_MAX_AGE))

    def test_tracked_callback_manifest_matches_runtime_configuration(self) -> None:
        manifest = json.loads((MODULE_PATH.parents[1] / "config" / "local-auth-callbacks.json").read_text(encoding="utf-8"))
        self.assertEqual(manifest["origin"], self.env["AUTH_ORIGIN"])
        for provider in ("google", "github", "microsoft", "amazon"):
            config = auth.provider_config(self.env, provider)
            assert config
            callbacks = (
                manifest["providers"][provider].get("redirect_uris")
                or manifest["providers"][provider].get("allowed_return_urls")
            )
            self.assertEqual(callbacks, [config["redirect_uri"]])
        self.assertEqual(manifest["session_cookie"]["max_age_seconds"], auth.SESSION_MAX_AGE)

    def test_authorization_urls_use_exact_callbacks_and_state(self) -> None:
        google = auth.provider_config(self.env, "google")
        github = auth.provider_config(self.env, "github")
        microsoft = auth.provider_config(self.env, "microsoft")
        amazon = auth.provider_config(self.env, "amazon")
        assert google and github and microsoft and amazon
        google_query = urllib.parse.parse_qs(urllib.parse.urlsplit(auth.authorization_url("google", google, "state-1")).query)
        github_query = urllib.parse.parse_qs(urllib.parse.urlsplit(auth.authorization_url("github", github, "state-2")).query)
        self.assertEqual(google_query["redirect_uri"], ["http://127.0.0.1:4173/auth/callback/google"])
        self.assertEqual(google_query["state"], ["state-1"])
        self.assertEqual(google_query["scope"], ["openid email profile"])
        self.assertEqual(github_query["redirect_uri"], ["http://127.0.0.1:4173/auth/callback/github"])
        self.assertEqual(github_query["state"], ["state-2"])
        self.assertIn("user:email", github_query["scope"][0])
        microsoft_query = urllib.parse.parse_qs(
            urllib.parse.urlsplit(auth.authorization_url("microsoft", microsoft, "state-3")).query
        )
        amazon_query = urllib.parse.parse_qs(
            urllib.parse.urlsplit(auth.authorization_url("amazon", amazon, "state-4")).query
        )
        self.assertEqual(microsoft_query["redirect_uri"], ["http://localhost:4173/auth/callback/microsoft"])
        self.assertEqual(microsoft_query["scope"], ["openid profile email"])
        self.assertEqual(microsoft_query["state"], ["state-3"])
        self.assertEqual(amazon_query["redirect_uri"], ["http://127.0.0.1:4173/auth/callback/amazon"])
        self.assertEqual(amazon_query["scope"], ["profile"])
        self.assertEqual(amazon_query["state"], ["state-4"])

    def test_google_exchange_returns_only_safe_profile_fields(self) -> None:
        config = auth.provider_config(self.env, "google")
        assert config
        with mock.patch.object(
            auth,
            "request_json",
            side_effect=[
                {"access_token": "provider-token", "refresh_token": "provider-refresh"},
                {
                    "sub": "abc",
                    "name": "A User",
                    "email": "a@example.com",
                    "email_verified": True,
                    "picture": "https://example.com/a.png",
                },
            ],
        ):
            user = auth.exchange_code("google", config, "one-time-code")
        self.assertEqual(user["sub"], "abc")
        self.assertNotIn("access_token", user)
        self.assertNotIn("refresh_token", user)

    def test_microsoft_and_amazon_exchange_normalize_provider_profiles(self) -> None:
        microsoft = auth.provider_config(self.env, "microsoft")
        amazon = auth.provider_config(self.env, "amazon")
        assert microsoft and amazon
        with mock.patch.object(
            auth,
            "request_json",
            side_effect=[
                {"access_token": "microsoft-token"},
                {"sub": "ms-sub", "name": "MS User", "email": "ms@example.com"},
            ],
        ):
            microsoft_user = auth.exchange_code("microsoft", microsoft, "microsoft-code")
        with mock.patch.object(
            auth,
            "request_json",
            side_effect=[
                {"access_token": "amazon-token"},
                {"user_id": "amzn-sub", "name": "Amazon User", "email": "amazon@example.com"},
            ],
        ):
            amazon_user = auth.exchange_code("amazon", amazon, "amazon-code")
        self.assertEqual(microsoft_user["provider"], "microsoft")
        self.assertEqual(microsoft_user["sub"], "ms-sub")
        self.assertEqual(amazon_user["provider"], "amazon")
        self.assertEqual(amazon_user["sub"], "amzn-sub")
        self.assertNotIn("access_token", microsoft_user)
        self.assertNotIn("access_token", amazon_user)

    def test_wallet_link_message_signature_and_private_store(self) -> None:
        wallet = auth.Account.create()
        address = wallet.address.lower()
        message = auth.wallet_link_message(
            "http://127.0.0.1:4173",
            "account-hash",
            address,
            "one-time-nonce",
            100,
            400,
        )
        signed = auth.Account.sign_message(auth.encode_defunct(text=message), wallet.key)
        signature = "0x" + signed.signature.hex()
        self.assertTrue(auth.verify_wallet_signature(message, signature, address))
        self.assertFalse(
            auth.verify_wallet_signature(message + "changed", signature, address)
        )
        self.assertIn("does not authorize a transaction", message)

        with tempfile.TemporaryDirectory() as temp_dir:
            store = auth.WalletLinkStore(Path(temp_dir) / "wallets.json", "w" * 48)
            user = {"provider": "google", "sub": "one"}
            wallets = store.link(user, address, "2026-08-23T00:00:00Z")
            self.assertEqual(wallets[0]["address"], address)
            self.assertEqual(store.wallets_for(user), wallets)
            with self.assertRaisesRegex(ValueError, "wallet_linked_to_another_account"):
                store.link({"provider": "github", "sub": "two"}, address, "2026-08-23T00:00:00Z")
            self.assertEqual(store.unlink(user, address), [])

    def test_linked_account_dashboard_uses_only_matching_canonical_evidence(self) -> None:
        wallet = "0x1111111111111111111111111111111111111111"
        other = "0x2222222222222222222222222222222222222222"
        evidence = {
            "autonomous": [
                {
                    "kind": "canonical_bounty_created",
                    "bounty_id": "0x" + "aa" * 32,
                    "data": {"creator": wallet},
                    "occurred_at": "2026-08-20T00:00:00Z",
                },
                {
                    "kind": "funding_added",
                    "bounty_id": "0x" + "aa" * 32,
                    "data": {"contributor": wallet, "amount": 2_000_000},
                    "occurred_at": "2026-08-20T00:00:01Z",
                },
                {
                    "kind": "bounty_settled",
                    "bounty_id": "0x" + "aa" * 32,
                    "data": {
                        "round": 1,
                        "solver": wallet,
                        "solver_reward": 1_500_000,
                        "timeout_bond_bonus": 250_000,
                    },
                    "occurred_at": "2026-08-21T00:00:00Z",
                },
                {
                    "kind": "bounty_claimed",
                    "bounty_id": "0x" + "bb" * 32,
                    "data": {"round": 2, "solver": wallet},
                    "occurred_at": "2026-08-22T00:00:00Z",
                },
                {
                    "kind": "funding_added",
                    "bounty_id": "0x" + "cc" * 32,
                    "data": {"contributor": other, "amount": 99_000_000},
                    "occurred_at": "2026-08-22T00:00:00Z",
                },
            ],
            "competition_v1": {"events": []},
            "competition_v2": {"events": []},
            "leaderboard": {
                "weekly": {
                    "ranking": {
                        "entries": [
                            {"rank": 3, "solver_wallet": wallet},
                            {"rank": 1, "solver_wallet": other},
                        ]
                    }
                }
            },
        }
        dashboard = auth.build_linked_account_dashboard(
            [{"address": wallet, "linked_at": "2026-08-23T00:00:00Z"}], evidence
        )
        self.assertEqual(dashboard["data_status"], "available")
        self.assertEqual(dashboard["stats"]["participating_bounties"], 1)
        self.assertEqual(dashboard["stats"]["completed_posted_bounties"], 1)
        self.assertEqual(dashboard["stats"]["earned_usdc"], "1.75")
        self.assertEqual(dashboard["stats"]["spent_usdc"], "2")
        self.assertEqual(dashboard["stats"]["leaderboard_rank"], 3)

    def test_local_http_routes_expose_provider_state_not_secrets(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            site = Path(temp_dir)
            (site / "index.html").write_text("ok", encoding="utf-8")
            handler = functools.partial(auth.LocalAuthHandler, directory=str(site))
            server = auth.LocalAuthServer(
                ("127.0.0.1", 0), handler, self.env, site / "wallet-links.json"
            )
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            try:
                connection = http.client.HTTPConnection("127.0.0.1", server.server_address[1], timeout=5)
                connection.request("GET", "/healthz")
                response = connection.getresponse()
                body = response.read().decode("utf-8")
                self.assertEqual(response.status, 200)
                self.assertIn('"google":true', body)
                self.assertNotIn("google-secret", body)

                connection.request("GET", "/auth/account")
                account_response = connection.getresponse()
                account_body = account_response.read().decode("utf-8")
                self.assertEqual(account_response.status, 401)
                self.assertIn('"error":"authentication_required"', account_body)

                connection.request("GET", "/auth/login/github")
                response = connection.getresponse()
                response.read()
                self.assertEqual(response.status, 302)
                location = response.getheader("Location")
                self.assertTrue(location.startswith("https://github.com/login/oauth/authorize?"))
                query = urllib.parse.parse_qs(urllib.parse.urlsplit(location).query)
                self.assertIn(query["state"][0], auth.OAUTH_STATES)
                connection.close()
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)

    def test_callback_sets_bounded_session_cookie_and_rejects_replay(self) -> None:
        with tempfile.TemporaryDirectory() as temp_dir:
            site = Path(temp_dir)
            (site / "index.html").write_text("ok", encoding="utf-8")
            handler = functools.partial(auth.LocalAuthHandler, directory=str(site))
            server = auth.LocalAuthServer(
                ("127.0.0.1", 0), handler, self.env, site / "wallet-links.json"
            )
            thread = threading.Thread(target=server.serve_forever, daemon=True)
            thread.start()
            state = "single-use-state"
            auth.OAUTH_STATES[state] = {"provider": "google", "created_at": auth.time.time()}
            profile = {
                "provider": "google",
                "sub": "google-subject",
                "name": "Test User",
                "email": "test@example.com",
                "avatar": "https://example.com/avatar.png",
            }
            try:
                connection = http.client.HTTPConnection("127.0.0.1", server.server_address[1], timeout=5)
                with mock.patch.object(auth, "exchange_code", return_value=profile):
                    connection.request("GET", f"/auth/callback/google?state={state}&code=one-time-code")
                    response = connection.getresponse()
                    response.read()
                self.assertEqual(response.status, 302)
                self.assertEqual(response.getheader("Location"), "/?auth=success&provider=google")
                cookie = response.getheader("Set-Cookie")
                self.assertIn("HttpOnly", cookie)
                self.assertIn("SameSite=Lax", cookie)
                self.assertNotIn("test@example.com", cookie)
                self.assertNotIn("one-time-code", cookie)

                session_cookie = cookie.split(";", 1)[0]
                connection.request("GET", "/auth/session", headers={"Cookie": session_cookie})
                session_response = connection.getresponse()
                session_body = session_response.read().decode("utf-8")
                self.assertEqual(session_response.status, 200)
                self.assertIn('"authenticated":true', session_body)
                self.assertIn('"provider":"google"', session_body)
                self.assertNotIn("provider-token", session_body)

                connection.request("GET", "/auth/account", headers={"Cookie": session_cookie})
                account_response = connection.getresponse()
                account_payload = json.loads(account_response.read().decode("utf-8"))
                self.assertEqual(account_response.status, 200)
                self.assertEqual(account_payload["data_status"], "unavailable")
                self.assertEqual(account_payload["reason"], "marketplace_identity_unlinked")
                self.assertIsNone(account_payload["stats"]["earned_usdc"])
                self.assertNotIn("email", account_payload)
                self.assertNotIn("sub", account_payload)

                wallet = auth.Account.create()
                wallet_address = wallet.address.lower()
                origin = f"http://127.0.0.1:{server.server_address[1]}"
                challenge_body = json.dumps({"address": wallet_address})
                connection.request(
                    "POST",
                    "/auth/wallet/challenge",
                    body=challenge_body,
                    headers={
                        "Content-Type": "application/json",
                        "Content-Length": str(len(challenge_body)),
                        "Cookie": session_cookie,
                        "Origin": origin,
                    },
                )
                challenge_response = connection.getresponse()
                challenge_payload = json.loads(challenge_response.read().decode("utf-8"))
                self.assertEqual(challenge_response.status, 201)
                self.assertIn("does not authorize a transaction", challenge_payload["message"])
                wallet_signature = "0x" + auth.Account.sign_message(
                    auth.encode_defunct(text=challenge_payload["message"]), wallet.key
                ).signature.hex()
                verify_body = json.dumps(
                    {
                        "challenge_id": challenge_payload["challenge_id"],
                        "address": wallet_address,
                        "signature": wallet_signature,
                    }
                )
                connection.request(
                    "POST",
                    "/auth/wallet/verify",
                    body=verify_body,
                    headers={
                        "Content-Type": "application/json",
                        "Content-Length": str(len(verify_body)),
                        "Cookie": session_cookie,
                        "Origin": origin,
                    },
                )
                verify_response = connection.getresponse()
                verify_payload = json.loads(verify_response.read().decode("utf-8"))
                self.assertEqual(verify_response.status, 200)
                self.assertTrue(verify_payload["linked"])
                self.assertEqual(verify_payload["wallets"][0]["address"], wallet_address)
                self.assertEqual(server.wallet_store.wallets_for(profile)[0]["address"], wallet_address)

                connection.request("GET", f"/auth/callback/google?state={state}&code=replayed-code")
                replay_response = connection.getresponse()
                replay_response.read()
                self.assertEqual(replay_response.status, 302)
                self.assertEqual(replay_response.getheader("Location"), "/?auth=error&reason=invalid_state")
                connection.close()
            finally:
                server.shutdown()
                server.server_close()
                thread.join(timeout=5)


if __name__ == "__main__":
    unittest.main()
