-- Badi: the grey writing preview must appear on the text line instantly.
-- Hyprland's default layer entrance animation makes it fade/slide in like a
-- floating tooltip, so the preview layer is exempt from animation.
hl.layer_rule({ name = "badi-preview-noanim", match = { namespace = "^badi-writing-preview$" }, no_anim = true })
