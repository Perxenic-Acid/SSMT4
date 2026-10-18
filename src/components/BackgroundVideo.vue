<script setup lang="ts">
import { onBeforeUnmount, onMounted, ref } from 'vue';
import { attachBackgroundVideo } from '../utils/backgroundVideo';

const props = defineProps<{ src: string; poster?: string }>();
const emit = defineEmits<{ failed: [source: string] }>();
const element = ref<HTMLVideoElement>();
let stop: (() => void) | undefined;
onMounted(() => {
  if (element.value) stop = attachBackgroundVideo(element.value, document, () => emit('failed', props.src));
});
onBeforeUnmount(() => stop?.());
</script>

<template>
  <video ref="element" :src="src" :poster="poster" loop muted playsinline preload="metadata" disablepictureinpicture />
</template>
