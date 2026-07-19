export type WorkflowItem = {
  href: string;
  label: string;
  title: string;
  step?: number;
  optional?: boolean;
  utility?: boolean;
};

export const WORKFLOW_STAGES: WorkflowItem[] = [
  { href: "/", label: "Setup", title: "Setup", step: 1 },
  { href: "/dictionary", label: "Dictionary", title: "Dictionary", step: 2 },
  { href: "/attribution", label: "Attribution", title: "Attribution", step: 3 },
  { href: "/harvest", label: "Harvest", title: "Harvest", step: 4 },
  { href: "/binding", label: "Binding", title: "Binding", step: 5 },
  { href: "/generation", label: "Generation", title: "Generation", step: 6 },
  { href: "/agent", label: "Review", title: "Dialogue Review", step: 7, optional: true },
  { href: "/export", label: "Export", title: "Export", step: 8 },
];

export const WORKFLOW_UTILITIES: WorkflowItem[] = [
  { href: "/transfer", label: "Transfer", title: "Transfer", utility: true },
];

export function workflowEyebrow(pathname: string): string {
  const item = [...WORKFLOW_STAGES, ...WORKFLOW_UTILITIES].find(({ href }) => href === pathname);
  if (!item) return "BG2 Voice Generator";
  if (item.utility) return "Profile utility";
  return `Step ${item.step} of ${WORKFLOW_STAGES.length}${item.optional ? " · Optional" : ""}`;
}
