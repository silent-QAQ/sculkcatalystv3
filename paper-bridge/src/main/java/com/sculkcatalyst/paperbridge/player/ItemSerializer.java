package com.sculkcatalyst.paperbridge.player;

import com.google.gson.JsonArray;
import com.google.gson.JsonNull;
import com.google.gson.JsonObject;
import com.sculkcatalyst.paperbridge.config.BridgeConfig;
import com.sculkcatalyst.paperbridge.text.Utf8TextLimiter;
import java.util.List;
import java.util.Objects;
import net.kyori.adventure.text.Component;
import net.kyori.adventure.text.serializer.plain.PlainTextComponentSerializer;
import org.bukkit.block.BlockState;
import org.bukkit.block.ShulkerBox;
import org.bukkit.inventory.ItemStack;
import org.bukkit.inventory.meta.BlockStateMeta;
import org.bukkit.inventory.meta.BundleMeta;
import org.bukkit.inventory.meta.ItemMeta;

/** Produces bounded display data only. Raw ItemStack serialization and NBT are intentionally forbidden. */
public final class ItemSerializer {
    private static final int MAX_CONTAINER_DEPTH = 3;
    private static final int MAX_TOTAL_ITEMS = 512;

    private final BridgeConfig.SnapshotLimits limits;
    private final PlainTextComponentSerializer textSerializer = PlainTextComponentSerializer.plainText();

    public ItemSerializer(BridgeConfig.SnapshotLimits limits) {
        this.limits = limits;
    }

    public JsonArray serializeSlots(ItemStack[] contents) {
        return serializeSlots(contents, 0, beginSnapshot(countNonAir(contents)));
    }

    public JsonArray serializeSlots(ItemStack[] contents, int firstSlot, SnapshotSession session) {
        Objects.requireNonNull(contents, "contents");
        Objects.requireNonNull(session, "session");

        JsonArray slots = new JsonArray();
        for (int index = 0; index < contents.length; index++) {
            slots.add(serializeSlot(firstSlot + index, contents[index], 0, true, session));
        }
        return slots;
    }

    public JsonObject serializeSlot(int slot, ItemStack item, SnapshotSession session) {
        Objects.requireNonNull(session, "session");
        return serializeSlot(slot, item, 0, true, session);
    }

    public JsonObject serializeItem(ItemStack item) {
        return serializeItem(item, 0, true, beginSnapshot(1));
    }

    public SnapshotSession beginSnapshot(int rootItemCount) {
        return new SnapshotSession(Math.max(0, MAX_TOTAL_ITEMS - Math.min(MAX_TOTAL_ITEMS, rootItemCount)));
    }

    private JsonObject serializeSlot(int slot, ItemStack item, int depth, boolean rootItem, SnapshotSession session) {
        JsonObject encoded = new JsonObject();
        encoded.addProperty("slot", slot);
        JsonObject serializedItem = serializeItem(item, depth, rootItem, session);
        encoded.add("item", serializedItem == null ? JsonNull.INSTANCE : serializedItem);
        return encoded;
    }

    private JsonObject serializeItem(ItemStack item, int depth, boolean rootItem, SnapshotSession session) {
        if (item == null || item.getType().isAir()) {
            return null;
        }
        if (!rootItem && !session.tryConsumeNestedItem()) {
            return null;
        }
        JsonObject result = new JsonObject();
        result.addProperty("id", item.getType().getKey().toString());
        result.addProperty("count", item.getAmount());

        if (!item.hasItemMeta()) {
            return result;
        }
        ItemMeta meta = item.getItemMeta();
        if (meta == null) {
            return result;
        }
        addDisplayData(result, meta);
        PreviewContent container = containerContent(meta);
        if (container != null && depth < MAX_CONTAINER_DEPTH) {
            result.add("container", serializeContainer(container, depth, session));
        }
        return result;
    }

    private void addDisplayData(JsonObject result, ItemMeta meta) {
        if (meta.hasCustomName() && meta.customName() != null) {
            result.addProperty("name", truncate(textSerializer.serialize(meta.customName())));
        }
        List<Component> lore = meta.lore();
        if (lore == null || lore.isEmpty()) {
            return;
        }
        JsonArray encodedLore = new JsonArray();
        int loreCount = Math.min(lore.size(), limits.maxLoreLines());
        for (int index = 0; index < loreCount; index++) {
            encodedLore.add(truncate(textSerializer.serialize(lore.get(index))));
        }
        result.add("lore", encodedLore);
    }

    private PreviewContent containerContent(ItemMeta meta) {
        if (meta instanceof BundleMeta bundleMeta && bundleMeta.hasItems()) {
            ItemStack[] contents = bundleMeta.getItems().toArray(ItemStack[]::new);
            return new PreviewContent("bundle", contents.length, contents);
        }
        if (meta instanceof BlockStateMeta blockStateMeta && blockStateMeta.hasBlockState()) {
            BlockState blockState = blockStateMeta.getBlockState();
            if (blockState instanceof ShulkerBox shulkerBox) {
                return new PreviewContent(
                    "shulker_box",
                    shulkerBox.getInventory().getSize(),
                    shulkerBox.getInventory().getContents()
                );
            }
        }
        return null;
    }

    private JsonObject serializeContainer(PreviewContent container, int depth, SnapshotSession session) {
        JsonObject encoded = new JsonObject();
        encoded.addProperty("kind", container.kind());
        encoded.addProperty("size", container.size());
        if (depth >= limits.maxPreviewDepth()) {
            encoded.add("slots", new JsonArray());
            return encoded;
        }
        JsonArray slots = new JsonArray();
        int maximum = Math.min(container.contents().length, limits.maxPreviewItems());
        for (int index = 0; index < maximum; index++) {
            slots.add(serializeSlot(index, container.contents()[index], depth + 1, false, session));
        }
        encoded.add("slots", slots);
        return encoded;
    }

    private static int countNonAir(ItemStack[] contents) {
        int count = 0;
        for (ItemStack item : contents) {
            if (item != null && !item.getType().isAir()) {
                count++;
            }
        }
        return count;
    }

    private String truncate(String value) {
        return Utf8TextLimiter.truncate(value == null ? "" : value, limits.maxTextLength());
    }

    public static final class SnapshotSession {
        private int remainingNestedItems;

        private SnapshotSession(int remainingNestedItems) {
            this.remainingNestedItems = remainingNestedItems;
        }

        private boolean tryConsumeNestedItem() {
            if (remainingNestedItems < 1) {
                return false;
            }
            remainingNestedItems--;
            return true;
        }
    }

    private record PreviewContent(String kind, int size, ItemStack[] contents) {
    }
}
