package com.pcremote.onboarding

import com.pcremote.R
import com.pcremote.agent.FailReason
import com.pcremote.agent.PairUri
import com.pcremote.agent.PairedPc
import com.pcremote.agent.PcInfo
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class PairingModelTest {
    private val fp = "b".repeat(64)
    private val qr = "pcremote://pair?n=Desk&h=192.168.0.9&p=47999&fp=$fp&t=otp"
    private val pc = PcInfo("pc-1", "DESK", macs = listOf("AA:BB"))

    private class Fakes(val agent: Result<AgentPaired>) {
        val stored = mutableListOf<PairedPc>()
        var pinSeenByAgent: String? = null
        val deps = PairingDeps(
            agentPair = { _: PairUri, _: String, pin: String -> pinSeenByAgent = pin; agent },
            save = { stored += it },
            device = "Pixel 8",
        )
    }

    private fun ok(host: String = "192.168.0.9") = Result.success(AgentPaired("tok", pc, host))
    private fun fail(r: FailReason) = Result.failure<AgentPaired>(AgentPairException(r))

    @Test fun `not a pc remote code`() = runTest {
        val f = Fakes(ok())
        val m = PairingModel(f.deps)
        m.onScanned("https://example.com")
        assertEquals(PairingState.Error(R.string.pcr_qr_invalid), m.state.value)
        assertTrue(f.stored.isEmpty())
    }

    @Test fun `expired code`() = runTest {
        val m = PairingModel(Fakes(fail(FailReason.BAD_OTP)).deps)
        m.onScanned(qr)
        assertEquals(PairingState.Error(R.string.pcr_qr_expired), m.state.value)
    }

    @Test fun `unreachable legacy code explains remote pairing limitation`() = runTest {
        val m = PairingModel(Fakes(fail(FailReason.UNREACHABLE)).deps)
        m.onScanned(qr)
        assertEquals(PairingState.Error(R.string.pcr_legacy_remote_pair), m.state.value)
    }

    @Test fun `cert mismatch is a security error and stores nothing`() = runTest {
        val f = Fakes(fail(FailReason.CERT_MISMATCH))
        val m = PairingModel(f.deps)
        m.onScanned(qr)
        assertEquals(PairingState.Error(R.string.pcr_cert_mismatch), m.state.value)
        assertTrue(f.stored.isEmpty())
    }

    @Test fun `success stores the pc with its token`() = runTest {
        val f = Fakes(ok())
        val m = PairingModel(f.deps)
        m.onScanned(qr)
        assertEquals(PairingState.Done("pc-1"), m.state.value)
        assertEquals(PairedPc("pc-1", "DESK", listOf("192.168.0.9"), 47999, fp, "tok", listOf("AA:BB")), f.stored.single())
        assertTrue(f.pinSeenByAgent!!.matches(Regex("""\d{4}""")))
    }

    @Test fun `remote pairing saves permanent relay credentials immediately`() = runTest {
        val relay = com.pcremote.agent.RelayInfo("wss://relay.example", pc.id, "device", ByteArray(32) { 1 })
        val f = Fakes(Result.success(AgentPaired("token", pc, "@relay", relay)))
        PairingModel(f.deps).onScanned(qr)
        assertEquals(relay, f.stored.single().relay)
        assertEquals(listOf("192.168.0.9"), f.stored.single().hosts)
    }
}
