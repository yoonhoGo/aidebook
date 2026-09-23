import { motion } from "motion/react";
import { SegmentedControl as RadixSegmentedControl } from "@radix-ui/themes";
import type { ReactNode } from "react";
import "./components.css";

export function PageMotion({ children, pageKey, reducedMotion }: { children: ReactNode; pageKey: string; reducedMotion: boolean }) {
  return <motion.div
    key={pageKey}
    className="page-motion"
    initial={reducedMotion ? false : { opacity: 0, y: 10 }}
    animate={{ opacity: 1, y: 0 }}
    transition={reducedMotion ? { duration: 0 } : { duration: 0.28, ease: [0.2, 0.8, 0.2, 1] }}
  >{children}</motion.div>;
}

export type SegmentOption<T extends string> = { value: T; label: string };

export function SegmentedControl<T extends string>({
  label, options, value, onChange, id, disabled = false, reducedMotion = false,
}: {
  label: string;
  options: readonly SegmentOption<T>[];
  value: T;
  onChange: (value: T) => void;
  id: string;
  disabled?: boolean;
  reducedMotion?: boolean;
}) {
  return <RadixSegmentedControl.Root
    className="app-segmented"
    aria-label={label}
    data-control-id={id}
    data-reduced-motion={reducedMotion || undefined}
    size="2"
    radius="medium"
    value={value}
    onValueChange={(next) => onChange(next as T)}
    disabled={disabled}
  >
    {options.map((option) => <RadixSegmentedControl.Item key={option.value} value={option.value} aria-label={option.label}>{option.label}</RadixSegmentedControl.Item>)}
  </RadixSegmentedControl.Root>;
}
