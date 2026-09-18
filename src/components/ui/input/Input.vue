<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { computed } from "vue";
import { cn } from "@/lib/utils";

const props = defineProps<{
  class?: HTMLAttributes["class"];
  modelValue?: string | number;
}>();

const emits = defineEmits<{
  (e: "update:modelValue", value: string | number): void;
}>();

const value = computed<string | number>({
  get: () => props.modelValue ?? "",
  set: (v) => emits("update:modelValue", v),
});
</script>

<template>
  <input
    v-model="value"
    :class="cn(
      'flex h-9 w-full min-w-0 rounded-md border border-input bg-transparent px-3 py-1 text-sm shadow-xs transition-[color,box-shadow] outline-none',
      'placeholder:text-muted-foreground selection:bg-primary selection:text-primary-foreground',
      'focus-visible:border-ring focus-visible:ring-ring/50 focus-visible:ring-[3px]',
      'disabled:cursor-not-allowed disabled:opacity-50',
      props.class,
    )"
  />
</template>
