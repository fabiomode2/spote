package dev.spote.app.remote

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test
import org.schabi.newpipe.extractor.MediaFormat
import org.schabi.newpipe.extractor.stream.AudioStream
import org.schabi.newpipe.extractor.stream.AudioTrackType
import org.schabi.newpipe.extractor.stream.DeliveryMethod

/**
 * JVM tests for the pure audio-stream ranking. No network and no Android framework involved:
 * [AudioStream] objects are built directly with NewPipeExtractor's public builder.
 */
class StreamSelectionTest {

    private fun audio(
        id: String,
        format: MediaFormat?,
        bitrate: Int = 0,
        deliveryMethod: DeliveryMethod = DeliveryMethod.DASH,
        url: String = "https://example.invalid/$id",
        isUrl: Boolean = true,
        trackType: AudioTrackType? = null,
    ): AudioStream = AudioStream.Builder()
        .setId(id)
        .setContent(url, isUrl)
        .setMediaFormat(format)
        .setDeliveryMethod(deliveryMethod)
        .setAverageBitrate(bitrate)
        .setAudioTrackType(trackType)
        .build()

    @Test
    fun `prefers aac m4a over opus when both have an adequate bitrate`() {
        val m4a = audio("140", MediaFormat.M4A, bitrate = 128)
        val opus = audio("251", MediaFormat.OPUS, bitrate = 160)

        assertEquals(m4a, AudioStreamSelector.select(listOf(opus, m4a)))
    }

    @Test
    fun `falls back to opus when aac only has a low bitrate`() {
        val lowBitrateAac = audio("139", MediaFormat.M4A, bitrate = 48)
        val opus = audio("251", MediaFormat.OPUS, bitrate = 160)

        assertEquals(opus, AudioStreamSelector.select(listOf(lowBitrateAac, opus)))
    }

    @Test
    fun `picks the highest bitrate inside the same codec family`() {
        val low = audio("250", MediaFormat.OPUS, bitrate = 64)
        val high = audio("251", MediaFormat.OPUS, bitrate = 160)

        assertEquals(high, AudioStreamSelector.select(listOf(low, high)))
    }

    @Test
    fun `prefers the original audio track over a dubbed one`() {
        val original = audio(
            id = "251",
            format = MediaFormat.OPUS,
            bitrate = 128,
            trackType = AudioTrackType.ORIGINAL,
        )
        val dubbed = audio(
            id = "140-dub",
            format = MediaFormat.M4A,
            bitrate = 192,
            trackType = AudioTrackType.DUBBED,
        )

        assertEquals(original, AudioStreamSelector.select(listOf(dubbed, original)))
    }

    @Test
    fun `skips manifests, non-url streams and video formats`() {
        val hlsManifest = audio(
            id = "hls",
            format = MediaFormat.M4A,
            bitrate = 128,
            deliveryMethod = DeliveryMethod.HLS,
            url = "https://example.invalid/master.m3u8",
        )
        val dashManifest = audio(
            id = "dash-manifest",
            format = MediaFormat.M4A,
            bitrate = 160,
            isUrl = false,
            url = "<MPD manifest content>",
        )
        val videoFormat = audio("18", MediaFormat.MPEG_4, bitrate = 96)
        val directAudio = audio("140", MediaFormat.M4A, bitrate = 128)

        assertEquals(
            directAudio,
            AudioStreamSelector.select(listOf(hlsManifest, dashManifest, videoFormat, directAudio)),
        )
    }

    @Test
    fun `returns null when there is no playable audio stream`() {
        assertNull(AudioStreamSelector.select(emptyList()))
        assertNull(
            AudioStreamSelector.select(
                listOf(
                    audio("video", MediaFormat.MPEG_4, bitrate = 96),
                    audio(
                        id = "manifest",
                        format = MediaFormat.M4A,
                        isUrl = false,
                        url = "<MPD manifest content>",
                    ),
                ),
            ),
        )
    }

    @Test
    fun `falls back to HLS when it is the only playable stream`() {
        val hls = audio(
            id = "hls",
            format = MediaFormat.M4A,
            bitrate = 128,
            deliveryMethod = DeliveryMethod.HLS,
            url = "https://example.invalid/stream.m3u8",
        )
        assertEquals(hls, AudioStreamSelector.select(listOf(hls)))
    }

    @Test
    fun `unknown bitrate still ranks by format`() {
        val opusUnknown = audio("251", MediaFormat.OPUS, bitrate = 0)
        val mp3Unknown = audio("mp3", MediaFormat.MP3, bitrate = 0)

        assertEquals(
            opusUnknown,
            AudioStreamSelector.select(listOf(mp3Unknown, opusUnknown)),
        )
    }
}
