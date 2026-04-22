.PHONY: serve build check deploy help

THEME   = quickstart
OUT     = site
CF_PROJECT = zetl-docs

help:
	@echo "make serve   # zetl serve with / → quickstart redirect (port 3000)"
	@echo "make build   # zetl build into ./$(OUT)/"
	@echo "make check   # zetl check (dead links, orphans, SPL diagnostics)"
	@echo "make deploy  # build + deploy to Cloudflare Pages ($(CF_PROJECT))"

serve:
	zetl -d . serve --theme $(THEME)

build:
	zetl -d . build --theme $(THEME) --out-dir $(OUT)

check:
	zetl -d . check

deploy: build
	wrangler pages deploy $(OUT) --project-name $(CF_PROJECT) --branch main --commit-dirty=true
