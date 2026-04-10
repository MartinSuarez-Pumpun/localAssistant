# Design System Strategy: The Sovereign Desktop

## 1. Overview & Creative North Star
The design system is guided by the Creative North Star: **"The Digital Architect."** 

This system rejects the ephemeral, "bubbly" nature of modern SaaS in favor of a retro-futuristic, institutional aesthetic. It draws inspiration from the mid-century precision of Olivetti hardware—where every lever and key felt intentional—and marries it with 2026 computational power. This is a "tool-not-toy" philosophy. 

We break the "template" look by utilizing **intentional asymmetry** and **tonal layering**. The UI is structured like a high-end editorial spread: heavy, authoritative typography balanced against vast, breathing white space. We move away from the "boxed-in" grid, instead using multi-column layouts where the content itself defines the boundaries, creating a sophisticated flow from raw source (left) to refined AI result (right).

---

## 2. Colors: Tonal Authority
The palette is rooted in institutional trust. It uses deep, commanding navies and "Action" blues to guide the user through complex document processing without visual fatigue.

### Palette Application
- **Primary (`#002542`):** Reserved for high-level navigation and institutional anchoring. Use `on_primary_container` (#66a6ea) for active states to ensure a "glowing" digital feel against deep backgrounds.
- **Tertiary/Action (`#401700` & `#C45911`):** Burnt orange is our surgical tool. Use it sparingly for alerts or critical "Process" actions. It provides a sharp, high-contrast heat signature against the cool navy and grey base.
- **Surface & Background:** We utilize `surface` (#f8f9fa) and `surface_container_lowest` (#ffffff) to distinguish between the "desk" (the UI) and the "paper" (the document).

### The "No-Line" Rule
**Prohibit 1px solid borders for sectioning.** To achieve a premium look, boundaries must be defined solely through background color shifts. For example, a side panel in `surface_container_low` should sit flush against a `surface` background. The change in hex value is the border.

### The "Glass & Gradient" Rule
To elevate the "Retro-Futurism" theme, floating palettes or context menus should utilize **Glassmorphism**. Apply a 20px backdrop-blur to `surface_container_highest` at 85% opacity. For primary action buttons, apply a subtle linear gradient from `primary` to `primary_container` to give the element a physical, tactile "sheen" reminiscent of polished bakelite.

---

## 3. Typography: The Editorial Engine
The typography is a dialogue between the machine (Sans) and the manuscript (Serif).

- **Display & Headlines (Inter):** Clean, geometric, and authoritative. These should be tracked slightly tighter (-2%) to feel like "set type." They represent the system's voice.
- **Body & Titles (Newsreader/Merriweather):** Used for the core product—the documents. This serif choice provides high legibility and an academic "heritage" feel. 
- **Labels (Inter):** Small-caps or high-weight labels in `label-sm` convey a technical, data-rich environment similar to a laboratory instrument.

---

## 4. Elevation & Depth: Tonal Layering
We do not use structural lines to separate data. We use **Physicality.**

### The Layering Principle
Depth is achieved by "stacking" surface-container tiers. 
1. **Base Layer:** `surface` (The environment).
2. **Secondary Workspaces:** `surface_container_low` (Sidebars and utility panels).
3. **Active Canvas:** `surface_container_lowest` (#FFFFFF) (The document or primary focus area).

### Ambient Shadows
When an element must float (e.g., a modal or a detached toolbar), use the **Ambient Shadow**:
- `box-shadow: 0 12px 32px rgba(25, 28, 29, 0.06);`
- The shadow must be large, diffused, and tinted with the `on_surface` color to mimic natural light hitting a heavy object.

### The "Ghost Border" Fallback
If a visual divider is required for accessibility, use a **Ghost Border**: The `outline_variant` token at **15% opacity**. This creates a suggestion of a boundary without cluttering the visual field.

---

## 5. Components: Precision Primitives

### Buttons
- **Primary:** High-contrast `primary` background. 0.25rem (sm) corner radius for a "square-set" professional look. Use `on_primary` for text.
- **Tertiary (Action):** `tertiary_container` with `on_tertiary_container` text. This is your "Processing" or "Execute" button.

### Inputs & Fields
- **Editorial Inputs:** No bottom line or box. Use a subtle `surface_container_high` background with a `sm` radius. Labels should always be `label-md` in `on_surface_variant`, positioned strictly above the field.

### Cards & Document Previews
- **Forbid dividers.** Separate metadata from body text using vertical whitespace (e.g., 24px) or a tonal shift in the card's background. 
- **The "Source-to-Result" Layout:** Use a multi-column view where the left column (Source) is `surface_dim` and the right column (AI Result) is `surface_container_lowest`, visually signaling the "cleaning" and "processing" of data as it moves right.

### Custom Component: The "Status Ribbon"
A 1.5px stroke element using `outline` that houses real-time "Offline" status and AI heartbeat. It should feel like a hardware label embossed on the UI.

---

## 6. Do's and Don'ts

### Do:
- **Use "Signature Asymmetry":** Align your primary document to the left but keep your AI insights in a floating, right-aligned "Glass" panel.
- **Embrace whitespace:** Treat every pixel of `surface` as valuable real estate that allows the user's mind to focus.
- **Use 1.5px Icons:** Stick strictly to 1.5px strokes. 1px is too spindly; 2px is too heavy. 1.5px is the "technical sweet spot."

### Don't:
- **Never use pure black (#000) for shadows.** It kills the "Institutional" sophistication.
- **Avoid 100% opaque borders.** If you can see the border before you see the content, it is too heavy.
- **No Rounded Corners > 8px:** This is an institutional tool. Large radii (pills/circles) feel too consumer-focused. Stick to the `sm` (0.125rem) and `md` (0.375rem) scales.