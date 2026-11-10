package com.pcremote.account

import androidx.test.platform.app.InstrumentationRegistry
import com.pcremote.agent.*
import kotlinx.coroutines.*
import kotlinx.coroutines.flow.filter
import kotlinx.coroutines.flow.first
import kotlinx.serialization.json.*
import org.junit.Assert.*
import org.junit.Test
import java.io.File

/** Scripted account service + actual Windows host pinned TLS, owner pairing and reconnect.
 * Run only with the explicitly started isolated Rust fixture. No installed host or user account. */
class AccountHostInstrumentedTest {
    @Test fun ownerBootstrapPairsAuthenticatesAndSignsOutThroughRealTls() = runBlocking {
        val ctx = InstrumentationRegistry.getInstrumentation().targetContext
        val fixtureFile = File(ctx.filesDir, "account-host-fixture.json")
        assertTrue("Start the isolated host fixture first", fixtureFile.isFile)
        val fixture = Json.parseToJsonElement(fixtureFile.readText()).jsonObject
        val fp = fixture.getValue("certificate_fp").jsonPrimitive.content
        val link = fixture.getValue("link").jsonPrimitive.content
        val owner = "11111111-1111-4111-8111-111111111111"
        val computer = "22222222-2222-4222-8222-222222222222"
        val request = "33333333-3333-4333-8333-333333333333"
        val expiry = System.currentTimeMillis() / 1000 + 110
        val transport = AccountTransport { req ->
            val body = when {
                req.url.endsWith("/v1/app/sign-in") -> buildJsonObject {
                    put("token", "a".repeat(64)); put("expires_at", System.currentTimeMillis() / 1000 + 3600)
                }
                req.url.endsWith("/v1/me") -> buildJsonObject { put("id", owner); put("handle", "review-user") }
                req.url.endsWith("/v1/computers") -> buildJsonArray { add(buildJsonObject {
                    put("id", computer); put("name", "Review PC"); put("platform", "windows")
                    put("certificate_fp", fp); put("linked", 1); put("online", 1)
                }) }
                req.url.endsWith("/v1/owner-pairings") -> buildJsonObject {
                    put("id", request); put("state", "pending"); put("expires", expiry)
                }
                req.url.endsWith("/v1/owner-pairings/$request") -> buildJsonObject {
                    put("id", request); put("link", link); put("certificate_fp", fp); put("expires", expiry)
                }
                req.url.endsWith("/auth/logout") -> JsonNull
                else -> error("Unexpected fixture request")
            }
            AccountResponse(if (body == JsonNull) 204 else 200, if (body == JsonNull) byteArrayOf() else body.toString().toByteArray())
        }
        val prefs = ctx.getSharedPreferences("account_bridge_test", 0)
        prefs.edit().clear().commit()
        val kv = object : KeyValue {
            override fun get(key: String) = prefs.getString(key, null)
            override fun put(key: String, value: String?) { check(prefs.edit().putString(key, value).commit()) }
        }
        val vault = AccountStore(kv, KeystoreBox)
        val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
        var connection: AgentConnection? = null
        try {
            val controller = withContext(Dispatchers.Main) {
                AccountController(vault, scope, "Android owner bridge", { AccountApi(it, transport) }, pair = { uri ->
                    val result = agentPair(scope, uri, "Android owner bridge", "0000", preferLan = true).getOrThrow()
                    PairedPc(result.pc.id, result.pc.name, uri.hosts, uri.port, uri.fingerprint, result.token, result.pc.macs)
                })
            }
            withTimeout(10_000) { controller.state.filter { it.ready }.first() }
            withContext(Dispatchers.Main) { controller.signIn("review@example.com", "correct horse") }
            val signedIn = withTimeout(15_000) { controller.state.filter { it.user != null && it.computers.isNotEmpty() }.first() }
            val opened = CompletableDeferred<String>()
            withContext(Dispatchers.Main) { controller.connect(signedIn.computers.single()) { opened.complete(it) } }
            val route = withTimeout(15_000) { opened.await() }
            val overlay = OwnedPcAccess(vault) { controller.state.value.user?.id }
            val combined = PcStore(kv, KeystoreBox, overlay)
            combined.upsert(PairedPc("pc", "Old manual pairing", emptyList(), 47999, fp, "revoked-manual-fixture", emptyList()))
            val pc = combined.find(route)
            assertTrue("Account Connect must resolve account credentials beside a manual pairing", pc?.accountOwner == owner)
            assertTrue("No account credential may overwrite the manual pairing", combined.find("pc")?.token == "revoked-manual-fixture")
            val conn = AgentConnection.forPc(pc!!, scope)
            connection = conn
            withContext(Dispatchers.Main) { conn.hello(pc.token, "Android owner bridge") }
            assertTrue("Stored account credential must authenticate with actual host", conn.state.value is AgentState.Online)
            val pong = scope.async(start = CoroutineStart.UNDISPATCHED) { conn.incoming.filter { it is Msg.Pong }.first() }
            conn.send(Msg.Ping)
            withTimeout(5000) { pong.await() }
            conn.close()
            withContext(Dispatchers.Main) { controller.signOut() }
            withTimeout(5000) { controller.state.filter { it.user == null && !it.busy }.first() }
            assertTrue("Owned route must disappear after sign out", combined.find(route) == null)
            assertTrue("Manual access must survive", combined.find("pc") != null)
            assertNull(vault.session())
        } finally {
            connection?.close(); scope.cancel(); prefs.edit().clear().commit(); fixtureFile.delete()
        }
    }
}
