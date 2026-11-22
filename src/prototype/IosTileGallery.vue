<script setup lang="ts">
// All the states of the iOS App tile in one column (`?scene=all`), each under its caption, inside the
// wizard's own page chrome classes so the tiles look exactly as they do on the remote page.
import IosTileProto from "./IosTileProto.vue";
import { GALLERY, SCENES } from "./setupIosScenes";
</script>

<template>
  <div class="settings setup-win iost-gallery">
    <div class="settings-body setup-body">
      <div class="setup-page">
        <section v-for="id in GALLERY" :key="id" class="iost-case">
          <p class="iost-case-caption">{{ SCENES[id].caption }}</p>
          <div class="channel-grid">
            <IosTileProto :facts="SCENES[id].facts" :agent-open="SCENES[id].agentOpen" />
          </div>
        </section>
      </div>
    </div>
  </div>
</template>
