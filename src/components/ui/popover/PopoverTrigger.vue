<script setup lang="ts">
import { PopoverTrigger } from "reka-ui";

const props = withDefaults(defineProps<{ asChild?: boolean }>(), { asChild: false });
</script>

<template>
  <PopoverTrigger :as-child="props.asChild">
    <slot />
  </PopoverTrigger>
</template>
