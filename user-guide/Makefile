.PHONY: serve build check deploy help

THEME   = quickstart
OUT     = site
CF_PROJECT = ztl-docs

help:
	@echo "make serve   # ztl serve with / → quickstart redirect (port 3000)"
	@echo "make build   # ztl build into ./$(OUT)/"
	@echo "make check   # ztl check (dead links, orphans, SPL diagnostics)"
	@echo "make deploy  # build + deploy to Cloudflare Pages ($(CF_PROJECT))"

serve:
	ztl -d . serve --theme $(THEME)

build:
	ztl -d . build --theme $(THEME) --out-dir $(OUT)

check:
	ztl -d . check

deploy: build
	wrangler pages deploy $(OUT) --project-name $(CF_PROJECT) --branch main --commit-dirty=true
