package com.sculkcatalyst.paperbridge.text;

import java.nio.charset.StandardCharsets;
import java.util.Objects;

/** Truncates display text without splitting a UTF-8 code point. */
public final class Utf8TextLimiter {
    private Utf8TextLimiter() {
    }

    public static int byteLength(String value) {
        return Objects.requireNonNull(value, "value").getBytes(StandardCharsets.UTF_8).length;
    }

    public static String truncate(String value, int maxUtf8Bytes) {
        Objects.requireNonNull(value, "value");
        if (maxUtf8Bytes < 0) {
            throw new IllegalArgumentException("maxUtf8Bytes must not be negative");
        }
        if (byteLength(value) <= maxUtf8Bytes) {
            return value;
        }
        StringBuilder truncated = new StringBuilder();
        int usedBytes = 0;
        for (int index = 0; index < value.length();) {
            int codePoint = value.codePointAt(index);
            String codePointText = new String(Character.toChars(codePoint));
            int codePointBytes = codePointText.getBytes(StandardCharsets.UTF_8).length;
            if (usedBytes + codePointBytes > maxUtf8Bytes) {
                break;
            }
            truncated.appendCodePoint(codePoint);
            usedBytes += codePointBytes;
            index += Character.charCount(codePoint);
        }
        return truncated.toString();
    }
}
