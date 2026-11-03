package app.lull.player

import androidx.media3.common.C
import androidx.media3.common.audio.AudioProcessor
import androidx.media3.common.audio.BaseAudioProcessor
import androidx.media3.common.util.UnstableApi
import app.tauri.annotation.InvokeArg
import java.nio.ByteBuffer
import kotlin.math.PI
import kotlin.math.cos
import kotlin.math.pow
import kotlin.math.sin
import kotlin.math.sqrt

/** One band of the web UI's equaliser (web/eq.js `spec`): `lowshelf`, `peaking` or `highshelf`. */
@InvokeArg
class EqBand {
    var type: String = "peaking"
    var hz: Double = 1000.0
    var gainDb: Double = 0.0
}

/** What the equaliser plays through: a level, then the bands in order. */
@InvokeArg
class EqSpec {
    var levelDb: Double = 0.0
    var bands: Array<EqBand> = emptyArray()
}

/**
 * The device's equaliser, shared by both decks: the web UI sets it (`equalize`), each deck's
 * [EqProcessor] reads it. Null means off: the processors pass the sound through untouched.
 */
object Equaliser {
    @Volatile
    var spec: EqSpec? = null
}

/**
 * The equaliser on PCM, as Web Audio's BiquadFilterNode computes it (its spec's formulas: peaks
 * with Q 1, shelves with slope 1), so the app sounds as the browser and Classic mode do. Runs on
 * the deck's playback thread; a new spec is picked up at the next buffer.
 */
@UnstableApi
class EqProcessor : BaseAudioProcessor() {
    private var built: EqSpec? = null
    private var gain = 1.0
    private var coeffs = emptyArray<DoubleArray>()

    /** Per channel and band: x1, x2, y1, y2. */
    private var memory = emptyArray<DoubleArray>()

    override fun onConfigure(inputAudioFormat: AudioProcessor.AudioFormat): AudioProcessor.AudioFormat {
        val pcm = inputAudioFormat.encoding == C.ENCODING_PCM_16BIT || inputAudioFormat.encoding == C.ENCODING_PCM_FLOAT
        return if (pcm) inputAudioFormat else AudioProcessor.AudioFormat.NOT_SET
    }

    override fun queueInput(inputBuffer: ByteBuffer) {
        val out = replaceOutputBuffer(inputBuffer.remaining())
        val spec = Equaliser.spec
        if (spec == null) {
            out.put(inputBuffer)
            out.flip()
            return
        }
        if (spec !== built) build(spec)
        val channels = inputAudioFormat.channelCount
        val float = inputAudioFormat.encoding == C.ENCODING_PCM_FLOAT
        var ch = 0
        while (inputBuffer.hasRemaining()) {
            val x = if (float) inputBuffer.float.toDouble() else inputBuffer.short / 32768.0
            val y = filter(ch, x * gain).coerceIn(-1.0, 1.0)
            if (float) out.putFloat(y.toFloat()) else out.putShort((y * 32767.0).toInt().toShort())
            ch = (ch + 1) % channels
        }
        out.flip()
    }

    override fun onFlush() {
        memory.forEach { it.fill(0.0) }
    }

    override fun onReset() {
        built = null
        coeffs = emptyArray()
        memory = emptyArray()
    }

    private fun filter(ch: Int, input: Double): Double {
        var x = input
        val m = memory[ch]
        for (b in coeffs.indices) {
            val k = coeffs[b]
            val o = b * 4
            val y = k[0] * x + k[1] * m[o] + k[2] * m[o + 1] - k[3] * m[o + 2] - k[4] * m[o + 3]
            m[o + 1] = m[o]
            m[o] = x
            m[o + 3] = m[o + 2]
            m[o + 2] = y
            x = y
        }
        return x
    }

    private fun build(spec: EqSpec) {
        val rate = inputAudioFormat.sampleRate.toDouble()
        val sameShape = coeffs.size == spec.bands.size && memory.size == inputAudioFormat.channelCount
        gain = 10.0.pow(spec.levelDb / 20.0)
        coeffs = Array(spec.bands.size) { coefficients(spec.bands[it], rate) }
        if (!sameShape) memory = Array(inputAudioFormat.channelCount) { DoubleArray(spec.bands.size * 4) }
        built = spec
    }

    /** b0, b1, b2, a1, a2, normalised by a0 (Web Audio's BiquadFilterNode). */
    private fun coefficients(band: EqBand, rate: Double): DoubleArray {
        val a = 10.0.pow(band.gainDb / 40.0)
        val w0 = 2 * PI * band.hz.coerceAtMost(rate / 2 - 1) / rate
        val c = cos(w0)
        val s = sin(w0)
        val k = when (band.type) {
            "lowshelf", "highshelf" -> {
                val r = 2 * sqrt(a) * (s / 2 * sqrt(2.0))
                if (band.type == "lowshelf") doubleArrayOf(
                    a * ((a + 1) - (a - 1) * c + r), 2 * a * ((a - 1) - (a + 1) * c), a * ((a + 1) - (a - 1) * c - r),
                    (a + 1) + (a - 1) * c + r, -2 * ((a - 1) + (a + 1) * c), (a + 1) + (a - 1) * c - r,
                ) else doubleArrayOf(
                    a * ((a + 1) + (a - 1) * c + r), -2 * a * ((a - 1) + (a + 1) * c), a * ((a + 1) + (a - 1) * c - r),
                    (a + 1) - (a - 1) * c + r, 2 * ((a - 1) - (a + 1) * c), (a + 1) - (a - 1) * c - r,
                )
            }
            else -> {
                val al = s / 2
                doubleArrayOf(1 + al * a, -2 * c, 1 - al * a, 1 + al / a, -2 * c, 1 - al / a)
            }
        }
        return doubleArrayOf(k[0] / k[3], k[1] / k[3], k[2] / k[3], k[4] / k[3], k[5] / k[3])
    }
}
