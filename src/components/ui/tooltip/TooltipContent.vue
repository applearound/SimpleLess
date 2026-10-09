<script setup lang="ts">
import type { HTMLAttributes } from "vue";
import { computed } from "vue";
import { TooltipContent, type TooltipContentEmits, type TooltipContentProps, TooltipPortal, useForwardPropsEmits } from "reka-ui";
import { cn } from "@/lib/utils";

const props = withDefaults(
  defineProps<TooltipContentProps & { class?: HTMLAttributes["class"] }>(),
  { sideOffset: 4 },
);
const emits = defineEmits<TooltipContentEmits>();

const delegatedProps = computed(() => {
  const { class: _, ...delegated } = props;
  return delegated;
});

const forwarded = useForwardPropsEmits(delegatedProps, emits);
</script>

<template>
  <TooltipPortal>
    <TooltipContent
      v-bind="forwarded"
      :class="cn(
        'z-50 max-w-96 break-all rounded-md border bg-popover px-3 py-1.5 text-xs text-popover-foreground shadow-md outline-none',
        props.class,
      )"
    >
      <slot />
    </TooltipContent>
  </TooltipPortal>
</template>
