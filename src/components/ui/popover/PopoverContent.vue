<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { PopoverContent, PopoverPortal } from "reka-ui";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<{
    class?: HTMLAttributes["class"];
    align?: "center" | "start" | "end";
    sideOffset?: number;
  }>(),
  { align: "center", sideOffset: 4 },
);
</script>

<template>
  <PopoverPortal>
    <PopoverContent
      :align="props.align"
      :side-offset="props.sideOffset"
      :class="cn(
        'z-50 w-72 rounded-md border bg-popover p-4 text-popover-foreground shadow-md outline-none',
        props.class,
      )"
    >
      <slot />
    </PopoverContent>
  </PopoverPortal>
</template>
