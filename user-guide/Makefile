.PHONY: serve build check help

THEME = quickstart
OUT   = site

help:
	@echo "make serve   # zetl serve with / → quickstart redirect (port 3000)"
	@echo "make build   # zetl build into ./$(OUT)/"
	@echo "make check   # zetl check (dead links, orphans, SPL diagnostics)"

serve:
	zetl -d . serve --theme $(THEME)

build:
	zetl -d . build --theme $(THEME) --out-dir $(OUT)

check:
	zetl -d . check
