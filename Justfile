# MIT/Apache2 License

protocol:
    cargo run --manifest-path=./xim-generator/Cargo.toml \
        i18nIMProto.c \
        ./xim-protocol/src/automatically_generated.rs \
        ./xim_protocol.toml