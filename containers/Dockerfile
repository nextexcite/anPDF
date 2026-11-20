FROM rust:1.100.0-slim-trixie@sha256:6f14b2b4b278ec0d50aa1d69541c304ce0f95e0f6268487aa9924af1c2e9aebe AS build
# Frozen Debian package universe, including transitive build inputs.
RUN rm -f /etc/apt/sources.list.d/* && printf 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261118T000000Z/ trixie main\ndeb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261118T000000Z/ trixie-updates main\ndeb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/20261118T000000Z/ trixie-security main\n' > /etc/apt/sources.list && apt-get update && apt-get install -y --no-install-recommends build-essential cmake curl ca-certificates libjpeg62-turbo-dev libssl-dev zlib1g-dev python3 && rm -rf /var/lib/apt/lists/*
WORKDIR /build
RUN curl --fail --location --proto '=https' https://github.com/qpdf/qpdf/releases/download/v12.4.2/qpdf-12.4.2.tar.gz -o /tmp/qpdf.tar.gz && echo '8a58af5b6141319287c1883bec8bd1bd545b7567b7fc5e6ce5d25a1c85f36397  /tmp/qpdf.tar.gz' | sha256sum -c - && tar -xzf /tmp/qpdf.tar.gz -C /tmp && cmake -S /tmp/qpdf-12.4.2 -B /tmp/qpdf-build -DCMAKE_BUILD_TYPE=Release -DCMAKE_INSTALL_PREFIX=/opt/qpdf -DCMAKE_INSTALL_LIBDIR=lib -DCMAKE_INSTALL_RPATH=/opt/qpdf/lib -DUSE_IMPLICIT_CRYPTO=OFF -DREQUIRE_CRYPTO_OPENSSL=ON -DDEFAULT_CRYPTO=openssl -DBUILD_STATIC_LIBS=OFF -DBUILD_SHARED_LIBS=ON -DBUILD_DOC=OFF -DINSTALL_EXAMPLES=OFF && cmake --build /tmp/qpdf-build --parallel 2 && cmake --install /tmp/qpdf-build
COPY Cargo.toml Cargo.lock build.rs ./
COPY build/forge-pin.json build/forge-pin.json
COPY build/get-pdfium.py build/get-pdfium.py
RUN python3 build/get-pdfium.py build/work/pdfium
COPY contracts contracts
COPY generated generated
COPY src src
ENV RUSTFLAGS="--remap-path-prefix=/build=/src --remap-path-prefix=/usr/local/cargo=/cargo"
RUN cargo install cargo-auditable --version 0.7.7 --locked && cargo auditable build --release --locked && /opt/qpdf/bin/qpdf --version

FROM debian:13.6-slim@sha256:d7e12182ce18b85b93007c1dedf31f2d29e01ccf3182cc4017c709b6259bc132
COPY --from=build /etc/ssl/certs/ca-certificates.crt /etc/ssl/certs/ca-certificates.crt
RUN rm -f /etc/apt/sources.list.d/* && printf 'deb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261118T000000Z/ trixie main\ndeb [check-valid-until=no] https://snapshot.debian.org/archive/debian/20261118T000000Z/ trixie-updates main\ndeb [check-valid-until=no] https://snapshot.debian.org/archive/debian-security/20261118T000000Z/ trixie-security main\n' > /etc/apt/sources.list && SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt apt-get -o Acquire::https::CaInfo=/etc/ssl/certs/ca-certificates.crt update && DEBIAN_FRONTEND=noninteractive SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt apt-get -o Acquire::https::CaInfo=/etc/ssl/certs/ca-certificates.crt upgrade -y && DEBIAN_FRONTEND=noninteractive SSL_CERT_FILE=/etc/ssl/certs/ca-certificates.crt apt-get -o Acquire::https::CaInfo=/etc/ssl/certs/ca-certificates.crt install -y --no-install-recommends ca-certificates libjpeg62-turbo libssl3t64 zlib1g && rm -rf /var/lib/apt/lists/* && mkdir -p /data/in /data/out && chmod 755 /data /data/in && chown 65532:65532 /data/out && chmod 700 /data/out
COPY --from=build /opt/qpdf /opt/qpdf
COPY --from=build /build/target/release/anpdf /usr/local/bin/anpdf
COPY --from=build /build/build/work/pdfium/lib/libpdfium.so /usr/local/lib/anpdf/libpdfium.so
COPY --from=build /build/build/work/pdfium/LICENSE /usr/share/licenses/pdfium/LICENSE
COPY --from=build /build/build/work/pdfium/licenses /usr/share/licenses/pdfium/licenses
COPY LICENSE NOTICE license-manifest.json /usr/share/licenses/anpdf/
COPY THIRD_PARTY_LICENSES /usr/share/licenses/anpdf/dependencies
COPY --from=build /tmp/qpdf-12.4.2/LICENSE.txt /usr/share/licenses/qpdf/LICENSE.txt
ARG APP_VERSION=0.3.0
ENV ANPDF_QPDF=/opt/qpdf/bin/qpdf
ENV ANPDF_PDFIUM_LIB=/usr/local/lib/anpdf/libpdfium.so
ENV ANPDF_QPDF_EXACT=1
LABEL org.opencontainers.image.title="anPDF" org.opencontainers.image.version="${APP_VERSION}" org.opencontainers.image.licenses="Apache-2.0"
USER 65532:65532
WORKDIR /data
ENTRYPOINT ["/usr/local/bin/anpdf"]
CMD ["operations"]
