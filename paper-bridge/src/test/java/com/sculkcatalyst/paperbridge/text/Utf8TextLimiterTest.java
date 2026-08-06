package com.sculkcatalyst.paperbridge.text;

import static org.junit.jupiter.api.Assertions.assertEquals;

import org.junit.jupiter.api.Test;

class Utf8TextLimiterTest {
    @Test
    void truncatesAtCompleteUtf8CodePoints() {
        assertEquals("你好", Utf8TextLimiter.truncate("你好世界", 6));
        assertEquals("", Utf8TextLimiter.truncate("你", 2));
        assertEquals("abc", Utf8TextLimiter.truncate("abcdef", 3));
        assertEquals(6, Utf8TextLimiter.byteLength("你好"));
    }
}
