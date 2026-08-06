package com.sculkcatalyst.paperbridge.player;

import com.google.gson.JsonArray;
import com.google.gson.JsonObject;
import java.util.HashSet;
import java.util.List;
import java.util.Locale;
import java.util.Set;
import org.bukkit.Location;
import org.bukkit.World;
import org.bukkit.entity.Player;
import org.bukkit.inventory.ItemStack;
import org.bukkit.inventory.PlayerInventory;

/**
 * Reads player state only from the player entity's scheduler context.
 * Scheduling is deliberately owned by FoliaTaskDispatcher rather than this class.
 */
public final class PlayerSnapshotService {
    private static final int PLAYER_STORAGE_SIZE = 36;
    private static final int ARMOR_SIZE = 4;
    private static final int ENDER_CHEST_SIZE = 27;
    private static final Set<String> ALL_SECTIONS = Set.of("basic", "inventory", "ender_chest");

    private final ItemSerializer itemSerializer;

    public PlayerSnapshotService(ItemSerializer itemSerializer) {
        this.itemSerializer = itemSerializer;
    }

    public JsonObject basicPlayer(Player player) {
        JsonObject result = new JsonObject();
        result.addProperty("uuid", player.getUniqueId().toString());
        result.addProperty("name", player.getName());
        result.addProperty("online", player.isOnline());
        result.addProperty("observed_at", System.currentTimeMillis());
        result.addProperty("level", player.getLevel());
        result.addProperty("experience_progress", player.getExp());
        result.addProperty("total_experience", player.getTotalExperience());
        result.addProperty("game_mode", player.getGameMode().name().toLowerCase(Locale.ROOT));
        result.addProperty("health", player.getHealth());
        result.addProperty("food_level", player.getFoodLevel());

        Location location = player.getLocation();
        World world = location.getWorld();
        if (world != null) {
            result.addProperty("dimension", world.getKey().toString());
        }
        result.add("position", position(location));
        return result;
    }

    public JsonObject snapshot(Player player, List<String> requestedSections) {
        Set<String> sections = normalizedSections(requestedSections);
        JsonObject result = basicPlayer(player);
        boolean includeInventory = sections.contains("inventory");
        ItemStack[] storageContents = new ItemStack[0];
        ItemStack[] armorContents = new ItemStack[0];
        ItemStack offHand = null;
        if (includeInventory) {
            PlayerInventory inventory = player.getInventory();
            storageContents = inventory.getStorageContents();
            armorContents = inventory.getArmorContents();
            offHand = inventory.getItemInOffHand();
        }

        ItemStack[] enderContents = new ItemStack[0];
        if (sections.contains("ender_chest")) {
            enderContents = player.getEnderChest().getContents();
        }
        ItemSerializer.SnapshotSession session = itemSerializer.beginSnapshot(
            countNonAir(storageContents, PLAYER_STORAGE_SIZE)
                + countNonAir(armorContents, ARMOR_SIZE)
                + countNonAir(offHand)
                + countNonAir(enderContents, ENDER_CHEST_SIZE)
        );

        if (includeInventory) {
            JsonObject inventory = new JsonObject();
            JsonArray slots = serializeSlots(storageContents, 0, PLAYER_STORAGE_SIZE, session);
            for (int index = 0; index < ARMOR_SIZE; index++) {
                slots.add(itemSerializer.serializeSlot(100 + index, itemAt(armorContents, index), session));
            }
            slots.add(itemSerializer.serializeSlot(-106, offHand, session));
            inventory.add("slots", slots);
            result.add("inventory", inventory);
        }
        if (sections.contains("ender_chest")) {
            JsonObject enderChest = new JsonObject();
            enderChest.add("slots", serializeSlots(enderContents, 0, ENDER_CHEST_SIZE, session));
            result.add("ender_chest", enderChest);
        }
        return result;
    }

    private static Set<String> normalizedSections(List<String> requestedSections) {
        if (requestedSections == null || requestedSections.isEmpty() || requestedSections.contains("all")) {
            return ALL_SECTIONS;
        }
        Set<String> accepted = new HashSet<>();
        for (String section : requestedSections) {
            if (section != null && ALL_SECTIONS.contains(section)) {
                accepted.add(section);
            }
        }
        return accepted;
    }

    private static JsonObject position(Location location) {
        JsonObject result = new JsonObject();
        result.addProperty("x", location.getX());
        result.addProperty("y", location.getY());
        result.addProperty("z", location.getZ());
        return result;
    }

    private JsonArray serializeSlots(
        ItemStack[] contents,
        int firstSlot,
        int size,
        ItemSerializer.SnapshotSession session
    ) {
        JsonArray slots = new JsonArray();
        for (int index = 0; index < size; index++) {
            slots.add(itemSerializer.serializeSlot(firstSlot + index, itemAt(contents, index), session));
        }
        return slots;
    }

    private static ItemStack itemAt(ItemStack[] contents, int index) {
        return index < contents.length ? contents[index] : null;
    }

    private static int countNonAir(ItemStack[] contents, int size) {
        int count = 0;
        for (int index = 0; index < size; index++) {
            count += countNonAir(itemAt(contents, index));
        }
        return count;
    }

    private static int countNonAir(ItemStack item) {
        return item != null && !item.getType().isAir() ? 1 : 0;
    }

}
