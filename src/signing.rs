// -*- mode: rust; -*-
//
// This file is part of ed25519-dalek.
// Copyright (c) 2017-2019 isis lovecruft
// See LICENSE for licensing information.
//
// Authors:
// - isis agora lovecruft <isis@patternsinthevoid.net>

//! ed25519 signing keys.

#[cfg(feature = "pkcs8")]
use ed25519::pkcs8;

#[cfg(any(test, feature = "rand_core"))]
use rand_core::CryptoRngCore;

#[cfg(feature = "serde")]
use serde::de::Error as SerdeError;
#[cfg(feature = "serde")]
use serde::{Deserialize, Deserializer, Serialize, Serializer};
#[cfg(feature = "serde")]
use serde_bytes::{ByteBuf as SerdeByteBuf, Bytes as SerdeBytes};

use sha2::Sha512;

#[cfg(feature = "digest")]
use curve25519_dalek::digest::generic_array::typenum::U64;
use curve25519_dalek::digest::Digest;
use curve25519_dalek::edwards::CompressedEdwardsY;
use curve25519_dalek::edwards::EdwardsPoint;
use curve25519_dalek::scalar::Scalar;

use ed25519::signature::{KeypairRef, Signer, Verifier};

#[cfg(feature = "digest")]
use crate::context::Context;
#[cfg(feature = "digest")]
use signature::DigestSigner;

#[cfg(feature = "zeroize")]
use zeroize::{Zeroize, ZeroizeOnDrop};

use crate::constants::*;
use crate::errors::*;
use crate::signature::*;
use crate::verifying::*;
use crate::Signature;

/// ed25519 secret key as defined in [RFC8032 § 5.1.5]:
///
/// > The private key is 32 octets (256 bits, corresponding to b) of
/// > cryptographically secure random data.
///
/// [RFC8032 § 5.1.5]: https://www.rfc-editor.org/rfc/rfc8032#section-5.1.5
pub type SecretKey = [u8; SECRET_KEY_LENGTH];

/// ed25519 signing key which can be used to produce signatures.
// Invariant: `public` is always the public key of `secret`. This prevents the signing function
// oracle attack described in https://github.com/MystenLabs/ed25519-unsafe-libs
#[derive(Clone, Debug)]
pub struct SigningKey {
    /// The secret half of this signing key.
    pub(crate) secret_key: SecretKey,
    /// The public half of this signing key.
    pub(crate) verifying_key: VerifyingKey,
}

/// # Example
///
/// ```
/// # extern crate ed25519_dalek;
/// #
/// use ed25519_dalek::SigningKey;
/// use ed25519_dalek::SECRET_KEY_LENGTH;
/// use ed25519_dalek::SignatureError;
///
/// # fn doctest() -> Result<SigningKey, SignatureError> {
/// let secret_key_bytes: [u8; SECRET_KEY_LENGTH] = [
///    157, 097, 177, 157, 239, 253, 090, 096,
///    186, 132, 074, 244, 146, 236, 044, 196,
///    068, 073, 197, 105, 123, 050, 105, 025,
///    112, 059, 172, 003, 028, 174, 127, 096, ];
///
/// let signing_key: SigningKey = SigningKey::from_bytes(&secret_key_bytes);
/// assert_eq!(signing_key.to_bytes(), secret_key_bytes);
///
/// # Ok(signing_key)
/// # }
/// #
/// # fn main() {
/// #     let result = doctest();
/// #     assert!(result.is_ok());
/// # }
/// ```
impl SigningKey {
    /// Construct a [`SigningKey`] from a [`SecretKey`]
    ///
    #[inline]
    pub fn from_bytes(secret_key: &SecretKey) -> Self {
        let verifying_key = VerifyingKey::from(&ExpandedSecretKey::from(secret_key));
        Self {
            secret_key: *secret_key,
            verifying_key,
        }
    }

    /// Convert this [`SigningKey`] into a [`SecretKey`]
    #[inline]
    pub fn to_bytes(&self) -> SecretKey {
        self.secret_key
    }

    /// Construct a [`SigningKey`] from the bytes of a `VerifyingKey` and `SecretKey`.
    ///
    /// # Inputs
    ///
    /// * `bytes`: an `&[u8]` of length [`KEYPAIR_LENGTH`], representing the
    ///   scalar for the secret key, and a compressed Edwards-Y coordinate of a
    ///   point on curve25519, both as bytes. (As obtained from
    ///   [`SigningKey::to_bytes`].)
    ///
    /// # Returns
    ///
    /// A `Result` whose okay value is an EdDSA [`SigningKey`] or whose error value
    /// is an `SignatureError` describing the error that occurred.
    #[inline]
    pub fn from_keypair_bytes(bytes: &[u8; 64]) -> Result<SigningKey, SignatureError> {
        let (secret_key, verifying_key) = bytes.split_at(SECRET_KEY_LENGTH);
        let signing_key = SigningKey::try_from(secret_key)?;
        let verifying_key = VerifyingKey::try_from(verifying_key)?;

        if signing_key.verifying_key() != verifying_key {
            return Err(InternalError::MismatchedKeypair.into());
        }

        Ok(signing_key)
    }

    /// Convert this signing key to a 64-byte keypair.
    ///
    /// # Returns
    ///
    /// An array of bytes, `[u8; KEYPAIR_LENGTH]`.  The first
    /// `SECRET_KEY_LENGTH` of bytes is the `SecretKey`, and the next
    /// `PUBLIC_KEY_LENGTH` bytes is the `VerifyingKey` (the same as other
    /// libraries, such as [Adam Langley's ed25519 Golang
    /// implementation](https://github.com/agl/ed25519/)). It is guaranteed that
    /// the encoded public key is the one derived from the encoded secret key.
    pub fn to_keypair_bytes(&self) -> [u8; KEYPAIR_LENGTH] {
        let mut bytes: [u8; KEYPAIR_LENGTH] = [0u8; KEYPAIR_LENGTH];

        bytes[..SECRET_KEY_LENGTH].copy_from_slice(&self.secret_key);
        bytes[SECRET_KEY_LENGTH..].copy_from_slice(self.verifying_key.as_bytes());
        bytes
    }

    /// Get the [`VerifyingKey`] for this [`SigningKey`].
    pub fn verifying_key(&self) -> VerifyingKey {
        self.verifying_key
    }

    /// Create a signing context that can be used for Ed25519ph with
    /// [`DigestSigner`].
    #[cfg(feature = "digest")]
    pub fn with_context<'k, 'v>(
        &'k self,
        context_value: &'v [u8],
    ) -> Result<Context<'k, 'v, Self>, SignatureError> {
        Context::new(self, context_value)
    }

    /// Generate an ed25519 signing key.
    ///
    /// # Example
    ///
    #[cfg_attr(feature = "rand_core", doc = "```")]
    #[cfg_attr(not(feature = "rand_core"), doc = "```ignore")]
    /// # fn main() {
    /// use rand::rngs::OsRng;
    /// use ed25519_dalek::{Signature, SigningKey};
    ///
    /// let mut csprng = OsRng;
    /// let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    /// # }
    /// ```
    ///
    /// # Input
    ///
    /// A CSPRNG with a `fill_bytes()` method, e.g. `rand_os::OsRng`.
    ///
    /// The caller must also supply a hash function which implements the
    /// `Digest` and `Default` traits, and which returns 512 bits of output.
    /// The standard hash function used for most ed25519 libraries is SHA-512,
    /// which is available with `use sha2::Sha512` as in the example above.
    /// Other suitable hash functions include Keccak-512 and Blake2b-512.
    #[cfg(any(test, feature = "rand_core"))]
    pub fn generate<R: CryptoRngCore + ?Sized>(csprng: &mut R) -> SigningKey {
        let mut secret = SecretKey::default();
        csprng.fill_bytes(&mut secret);
        Self::from_bytes(&secret)
    }

    /// Sign a `prehashed_message` with this [`SigningKey`] using the
    /// Ed25519ph algorithm defined in [RFC8032 §5.1][rfc8032].
    ///
    /// # Inputs
    ///
    /// * `prehashed_message` is an instantiated SHA-512 digest of the message
    /// * `context` is an optional context string, up to 255 bytes inclusive,
    ///   which may be used to provide additional domain separation.  If not
    ///   set, this will default to an empty string.
    ///
    /// # Returns
    ///
    /// An Ed25519ph [`Signature`] on the `prehashed_message`.
    ///
    /// # Note
    ///
    /// The RFC only permits SHA-512 to be used for prehashing. This function technically works,
    /// and is probably safe to use, with any secure hash function with 512-bit digests, but
    /// anything outside of SHA-512 is NOT specification-compliant. We expose [`crate::Sha512`] for
    /// user convenience.
    ///
    /// # Examples
    ///
    #[cfg_attr(all(feature = "rand_core", feature = "digest"), doc = "```")]
    #[cfg_attr(
        any(not(feature = "rand_core"), not(feature = "digest")),
        doc = "```ignore"
    )]
    /// use ed25519_dalek::Digest;
    /// use ed25519_dalek::SigningKey;
    /// use ed25519_dalek::Signature;
    /// use sha2::Sha512;
    /// use rand::rngs::OsRng;
    ///
    /// # fn main() {
    /// let mut csprng = OsRng;
    /// let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    /// let message: &[u8] = b"All I want is to pet all of the dogs.";
    ///
    /// // Create a hash digest object which we'll feed the message into:
    /// let mut prehashed: Sha512 = Sha512::new();
    ///
    /// prehashed.update(message);
    /// # }
    /// ```
    ///
    /// If you want, you can optionally pass a "context".  It is generally a
    /// good idea to choose a context and try to make it unique to your project
    /// and this specific usage of signatures.
    ///
    /// For example, without this, if you were to [convert your OpenPGP key
    /// to a Bitcoin key][terrible_idea] (just as an example, and also Don't
    /// Ever Do That) and someone tricked you into signing an "email" which was
    /// actually a Bitcoin transaction moving all your magic internet money to
    /// their address, it'd be a valid transaction.
    ///
    /// By adding a context, this trick becomes impossible, because the context
    /// is concatenated into the hash, which is then signed.  So, going with the
    /// previous example, if your bitcoin wallet used a context of
    /// "BitcoinWalletAppTxnSigning" and OpenPGP used a context (this is likely
    /// the least of their safety problems) of "GPGsCryptoIsntConstantTimeLol",
    /// then the signatures produced by both could never match the other, even
    /// if they signed the exact same message with the same key.
    ///
    /// Let's add a context for good measure (remember, you'll want to choose
    /// your own!):
    ///
    #[cfg_attr(all(feature = "rand_core", feature = "digest"), doc = "```")]
    #[cfg_attr(
        any(not(feature = "rand_core"), not(feature = "digest")),
        doc = "```ignore"
    )]
    /// # use ed25519_dalek::Digest;
    /// # use ed25519_dalek::SigningKey;
    /// # use ed25519_dalek::Signature;
    /// # use ed25519_dalek::SignatureError;
    /// # use sha2::Sha512;
    /// # use rand::rngs::OsRng;
    /// #
    /// # fn do_test() -> Result<Signature, SignatureError> {
    /// # let mut csprng = OsRng;
    /// # let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    /// # let message: &[u8] = b"All I want is to pet all of the dogs.";
    /// # let mut prehashed: Sha512 = Sha512::new();
    /// # prehashed.update(message);
    /// #
    /// let context: &[u8] = b"Ed25519DalekSignPrehashedDoctest";
    ///
    /// let sig: Signature = signing_key.sign_prehashed(prehashed, Some(context))?;
    /// #
    /// # Ok(sig)
    /// # }
    /// # fn main() {
    /// #     do_test();
    /// # }
    /// ```
    ///
    /// [rfc8032]: https://tools.ietf.org/html/rfc8032#section-5.1
    /// [terrible_idea]: https://github.com/isislovecruft/scripts/blob/master/gpgkey2bc.py
    #[cfg(feature = "digest")]
    pub fn sign_prehashed<D>(
        &self,
        prehashed_message: D,
        context: Option<&[u8]>,
    ) -> Result<Signature, SignatureError>
    where
        D: Digest<OutputSize = U64>,
    {
        let expanded: ExpandedSecretKey = (&self.secret_key).into(); // xxx thanks i hate this

        expanded.sign_prehashed(prehashed_message, &self.verifying_key, context)
    }

    /// Verify a signature on a message with this signing key's public key.
    pub fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), SignatureError> {
        self.verifying_key.verify(message, signature)
    }

    /// Verify a `signature` on a `prehashed_message` using the Ed25519ph algorithm.
    ///
    /// # Inputs
    ///
    /// * `prehashed_message` is an instantiated hash digest with 512-bits of
    ///   output which has had the message to be signed previously fed into its
    ///   state.
    /// * `context` is an optional context string, up to 255 bytes inclusive,
    ///   which may be used to provide additional domain separation.  If not
    ///   set, this will default to an empty string.
    /// * `signature` is a purported Ed25519ph [`Signature`] on the `prehashed_message`.
    ///
    /// # Returns
    ///
    /// Returns `true` if the `signature` was a valid signature created by this
    /// [`SigningKey`] on the `prehashed_message`.
    ///
    /// # Examples
    ///
    #[cfg_attr(all(feature = "rand_core", feature = "digest"), doc = "```")]
    #[cfg_attr(
        any(not(feature = "rand_core"), not(feature = "digest")),
        doc = "```ignore"
    )]
    /// use ed25519_dalek::Digest;
    /// use ed25519_dalek::SigningKey;
    /// use ed25519_dalek::Signature;
    /// use ed25519_dalek::SignatureError;
    /// use sha2::Sha512;
    /// use rand::rngs::OsRng;
    ///
    /// # fn do_test() -> Result<(), SignatureError> {
    /// let mut csprng = OsRng;
    /// let signing_key: SigningKey = SigningKey::generate(&mut csprng);
    /// let message: &[u8] = b"All I want is to pet all of the dogs.";
    ///
    /// let mut prehashed: Sha512 = Sha512::new();
    /// prehashed.update(message);
    ///
    /// let context: &[u8] = b"Ed25519DalekSignPrehashedDoctest";
    ///
    /// let sig: Signature = signing_key.sign_prehashed(prehashed, Some(context))?;
    ///
    /// // The sha2::Sha512 struct doesn't implement Copy, so we'll have to create a new one:
    /// let mut prehashed_again: Sha512 = Sha512::default();
    /// prehashed_again.update(message);
    ///
    /// let verified = signing_key.verifying_key().verify_prehashed(prehashed_again, Some(context), &sig);
    ///
    /// assert!(verified.is_ok());
    ///
    /// # verified
    /// # }
    /// #
    /// # fn main() {
    /// #     do_test();
    /// # }
    /// ```
    ///
    /// [rfc8032]: https://tools.ietf.org/html/rfc8032#section-5.1
    #[cfg(feature = "digest")]
    pub fn verify_prehashed<D>(
        &self,
        prehashed_message: D,
        context: Option<&[u8]>,
        signature: &Signature,
    ) -> Result<(), SignatureError>
    where
        D: Digest<OutputSize = U64>,
    {
        self.verifying_key
            .verify_prehashed(prehashed_message, context, signature)
    }

    /// Strictly verify a signature on a message with this signing key's public key.
    ///
    /// # On The (Multiple) Sources of Malleability in Ed25519 Signatures
    ///
    /// This version of verification is technically non-RFC8032 compliant.  The
    /// following explains why.
    ///
    /// 1. Scalar Malleability
    ///
    /// The authors of the RFC explicitly stated that verification of an ed25519
    /// signature must fail if the scalar `s` is not properly reduced mod \ell:
    ///
    /// > To verify a signature on a message M using public key A, with F
    /// > being 0 for Ed25519ctx, 1 for Ed25519ph, and if Ed25519ctx or
    /// > Ed25519ph is being used, C being the context, first split the
    /// > signature into two 32-octet halves.  Decode the first half as a
    /// > point R, and the second half as an integer S, in the range
    /// > 0 <= s < L.  Decode the public key A as point A'.  If any of the
    /// > decodings fail (including S being out of range), the signature is
    /// > invalid.)
    ///
    /// All `verify_*()` functions within ed25519-dalek perform this check.
    ///
    /// 2. Point malleability
    ///
    /// The authors of the RFC added in a malleability check to step #3 in
    /// §5.1.7, for small torsion components in the `R` value of the signature,
    /// *which is not strictly required*, as they state:
    ///
    /// > Check the group equation \[8\]\[S\]B = \[8\]R + \[8\]\[k\]A'.  It's
    /// > sufficient, but not required, to instead check \[S\]B = R + \[k\]A'.
    ///
    /// # History of Malleability Checks
    ///
    /// As originally defined (cf. the "Malleability" section in the README of
    /// this repo), ed25519 signatures didn't consider *any* form of
    /// malleability to be an issue.  Later the scalar malleability was
    /// considered important.  Still later, particularly with interests in
    /// cryptocurrency design and in unique identities (e.g. for Signal users,
    /// Tor onion services, etc.), the group element malleability became a
    /// concern.
    ///
    /// However, libraries had already been created to conform to the original
    /// definition.  One well-used library in particular even implemented the
    /// group element malleability check, *but only for batch verification*!
    /// Which meant that even using the same library, a single signature could
    /// verify fine individually, but suddenly, when verifying it with a bunch
    /// of other signatures, the whole batch would fail!
    ///
    /// # "Strict" Verification
    ///
    /// This method performs *both* of the above signature malleability checks.
    ///
    /// It must be done as a separate method because one doesn't simply get to
    /// change the definition of a cryptographic primitive ten years
    /// after-the-fact with zero consideration for backwards compatibility in
    /// hardware and protocols which have it already have the older definition
    /// baked in.
    ///
    /// # Return
    ///
    /// Returns `Ok(())` if the signature is valid, and `Err` otherwise.
    #[allow(non_snake_case)]
    pub fn verify_strict(
        &self,
        message: &[u8],
        signature: &Signature,
    ) -> Result<(), SignatureError> {
        self.verifying_key.verify_strict(message, signature)
    }
}

impl AsRef<VerifyingKey> for SigningKey {
    fn as_ref(&self) -> &VerifyingKey {
        &self.verifying_key
    }
}

impl KeypairRef for SigningKey {
    type VerifyingKey = VerifyingKey;
}

impl Signer<Signature> for SigningKey {
    /// Sign a message with this signing key's secret key.
    fn try_sign(&self, message: &[u8]) -> Result<Signature, SignatureError> {
        let expanded: ExpandedSecretKey = (&self.secret_key).into();
        Ok(expanded.sign(message, &self.verifying_key))
    }
}

/// Equivalent to [`SigningKey::sign_prehashed`] with `context` set to [`None`].
///
/// # Note
///
/// The RFC only permits SHA-512 to be used for prehashing. This function technically works, and is
/// probably safe to use, with any secure hash function with 512-bit digests, but anything outside
/// of SHA-512 is NOT specification-compliant. We expose [`crate::Sha512`] for user convenience.
#[cfg(feature = "digest")]
impl<D> DigestSigner<D, Signature> for SigningKey
where
    D: Digest<OutputSize = U64>,
{
    fn try_sign_digest(&self, msg_digest: D) -> Result<Signature, SignatureError> {
        self.sign_prehashed(msg_digest, None)
    }
}

/// Equivalent to [`SigningKey::sign_prehashed`] with `context` set to [`Some`]
/// containing `self.value()`.
///
/// # Note
///
/// The RFC only permits SHA-512 to be used for prehashing. This function technically works, and is
/// probably safe to use, with any secure hash function with 512-bit digests, but anything outside
/// of SHA-512 is NOT specification-compliant. We expose [`crate::Sha512`] for user convenience.
#[cfg(feature = "digest")]
impl<D> DigestSigner<D, Signature> for Context<'_, '_, SigningKey>
where
    D: Digest<OutputSize = U64>,
{
    fn try_sign_digest(&self, msg_digest: D) -> Result<Signature, SignatureError> {
        self.key().sign_prehashed(msg_digest, Some(self.value()))
    }
}

impl Verifier<Signature> for SigningKey {
    /// Verify a signature on a message with this signing key's public key.
    fn verify(&self, message: &[u8], signature: &Signature) -> Result<(), SignatureError> {
        self.verifying_key.verify(message, signature)
    }
}

impl From<SecretKey> for SigningKey {
    #[inline]
    fn from(secret: SecretKey) -> Self {
        Self::from_bytes(&secret)
    }
}

impl From<&SecretKey> for SigningKey {
    #[inline]
    fn from(secret: &SecretKey) -> Self {
        Self::from_bytes(secret)
    }
}

impl TryFrom<&[u8]> for SigningKey {
    type Error = SignatureError;

    fn try_from(bytes: &[u8]) -> Result<SigningKey, SignatureError> {
        SecretKey::try_from(bytes)
            .map(|bytes| Self::from_bytes(&bytes))
            .map_err(|_| {
                InternalError::BytesLength {
                    name: "SecretKey",
                    length: SECRET_KEY_LENGTH,
                }
                .into()
            })
    }
}

#[cfg(feature = "zeroize")]
impl Drop for SigningKey {
    fn drop(&mut self) {
        self.secret_key.zeroize();
    }
}

#[cfg(feature = "zeroize")]
impl ZeroizeOnDrop for SigningKey {}

#[cfg(all(feature = "alloc", feature = "pkcs8"))]
impl pkcs8::EncodePrivateKey for SigningKey {
    fn to_pkcs8_der(&self) -> pkcs8::Result<pkcs8::SecretDocument> {
        pkcs8::KeypairBytes::from(self).to_pkcs8_der()
    }
}

#[cfg(feature = "pkcs8")]
impl TryFrom<pkcs8::KeypairBytes> for SigningKey {
    type Error = pkcs8::Error;

    fn try_from(pkcs8_key: pkcs8::KeypairBytes) -> pkcs8::Result<Self> {
        SigningKey::try_from(&pkcs8_key)
    }
}

#[cfg(feature = "pkcs8")]
impl TryFrom<&pkcs8::KeypairBytes> for SigningKey {
    type Error = pkcs8::Error;

    fn try_from(pkcs8_key: &pkcs8::KeypairBytes) -> pkcs8::Result<Self> {
        let signing_key = SigningKey::from_bytes(&pkcs8_key.secret_key);

        // Validate the public key in the PKCS#8 document if present
        if let Some(public_bytes) = &pkcs8_key.public_key {
            let expected_verifying_key = VerifyingKey::from_bytes(public_bytes.as_ref())
                .map_err(|_| pkcs8::Error::KeyMalformed)?;

            if signing_key.verifying_key() != expected_verifying_key {
                return Err(pkcs8::Error::KeyMalformed);
            }
        }

        Ok(signing_key)
    }
}

#[cfg(feature = "pkcs8")]
impl From<SigningKey> for pkcs8::KeypairBytes {
    fn from(signing_key: SigningKey) -> pkcs8::KeypairBytes {
        pkcs8::KeypairBytes::from(&signing_key)
    }
}

#[cfg(feature = "pkcs8")]
impl From<&SigningKey> for pkcs8::KeypairBytes {
    fn from(signing_key: &SigningKey) -> pkcs8::KeypairBytes {
        pkcs8::KeypairBytes {
            secret_key: signing_key.to_bytes(),
            public_key: Some(pkcs8::PublicKeyBytes(signing_key.verifying_key.to_bytes())),
        }
    }
}

#[cfg(feature = "pkcs8")]
impl TryFrom<pkcs8::PrivateKeyInfo<'_>> for SigningKey {
    type Error = pkcs8::Error;

    fn try_from(private_key: pkcs8::PrivateKeyInfo<'_>) -> pkcs8::Result<Self> {
        pkcs8::KeypairBytes::try_from(private_key)?.try_into()
    }
}

#[cfg(feature = "serde")]
impl Serialize for SigningKey {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        SerdeBytes::new(&self.secret_key).serialize(serializer)
    }
}

#[cfg(feature = "serde")]
impl<'d> Deserialize<'d> for SigningKey {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'d>,
    {
        let bytes = <SerdeByteBuf>::deserialize(deserializer)?;
        Self::try_from(bytes.as_ref()).map_err(SerdeError::custom)
    }
}

/// An "expanded" secret key.
///
/// This is produced by using an hash function with 512-bits output to digest a
/// `SecretKey`.  The output digest is then split in half, the lower half being
/// the actual `key` used to sign messages, after twiddling with some bits.¹ The
/// upper half is used a sort of half-baked, ill-designed² pseudo-domain-separation
/// "nonce"-like thing, which is used during signature production by
/// concatenating it with the message to be signed before the message is hashed.
///
/// Instances of this secret are automatically overwritten with zeroes when they
/// fall out of scope.
//
// ¹ This results in a slight bias towards non-uniformity at one spectrum of
// the range of valid keys.  Oh well: not my idea; not my problem.
//
// ² It is the author's view (specifically, isis agora lovecruft, in the event
// you'd like to complain about me, again) that this is "ill-designed" because
// this doesn't actually provide true hash domain separation, in that in many
// real-world applications a user wishes to have one key which is used in
// several contexts (such as within tor, which does domain separation
// manually by pre-concatenating static strings to messages to achieve more
// robust domain separation).  In other real-world applications, such as
// bitcoind, a user might wish to have one master keypair from which others are
// derived (à la BIP32) and different domain separators between keys derived at
// different levels (and similarly for tree-based key derivation constructions,
// such as hash-based signatures).  Leaving the domain separation to
// application designers, who thus far have produced incompatible,
// slightly-differing, ad hoc domain separation (at least those application
// designers who knew enough cryptographic theory to do so!), is therefore a
// bad design choice on the part of the cryptographer designing primitives
// which should be simple and as foolproof as possible to use for
// non-cryptographers.  Further, later in the ed25519 signature scheme, as
// specified in RFC8032, the public key is added into *another* hash digest
// (along with the message, again); it is unclear to this author why there's
// not only one but two poorly-thought-out attempts at domain separation in the
// same signature scheme, and which both fail in exactly the same way.  For a
// better-designed, Schnorr-based signature scheme, see Trevor Perrin's work on
// "generalised EdDSA" and "VXEdDSA".
pub(crate) struct ExpandedSecretKey {
    pub(crate) key: Scalar,
    pub(crate) nonce: [u8; 32],
}

#[cfg(feature = "zeroize")]
impl Drop for ExpandedSecretKey {
    fn drop(&mut self) {
        self.key.zeroize();
        self.nonce.zeroize()
    }
}

impl From<&SecretKey> for ExpandedSecretKey {
    #[allow(clippy::unwrap_used)]
    fn from(secret_key: &SecretKey) -> ExpandedSecretKey {
        let hash = Sha512::default().chain_update(secret_key).finalize();
        // TODO: Use bytes.split_array_ref once it’s in MSRV.
        let (lower, upper) = hash.split_at(32);

        // The try_into here converts to fixed-size array
        ExpandedSecretKey {
            key: Scalar::from_bits_clamped(lower.try_into().unwrap()),
            nonce: upper.try_into().unwrap(),
        }
    }
}

impl ExpandedSecretKey {
    /// Sign a message with this `ExpandedSecretKey`.
    #[allow(non_snake_case)]
    pub(crate) fn sign(&self, message: &[u8], verifying_key: &VerifyingKey) -> Signature {
        let mut h: Sha512 = Sha512::new();

        h.update(self.nonce);
        h.update(message);

        let r = Scalar::from_hash(h);
        let R: CompressedEdwardsY = EdwardsPoint::mul_base(&r).compress();

        h = Sha512::new();
        h.update(R.as_bytes());
        h.update(verifying_key.as_bytes());
        h.update(message);

        let k = Scalar::from_hash(h);
        let s: Scalar = (k * self.key) + r;

        InternalSignature { R, s }.into()
    }

    /// Sign a `prehashed_message` with this `ExpandedSecretKey` using the
    /// Ed25519ph algorithm defined in [RFC8032 §5.1][rfc8032].
    ///
    /// # Inputs
    ///
    /// * `prehashed_message` is an instantiated hash digest with 512-bits of
    ///   output which has had the message to be signed previously fed into its
    ///   state.
    /// * `verifying_key` is a [`VerifyingKey`] which corresponds to this secret key.
    /// * `context` is an optional context string, up to 255 bytes inclusive,
    ///   which may be used to provide additional domain separation.  If not
    ///   set, this will default to an empty string.
    ///
    /// # Returns
    ///
    /// A `Result` whose `Ok` value is an Ed25519ph [`Signature`] on the
    /// `prehashed_message` if the context was 255 bytes or less, otherwise
    /// a `SignatureError`.
    ///
    /// [rfc8032]: https://tools.ietf.org/html/rfc8032#section-5.1
    #[cfg(feature = "digest")]
    #[allow(non_snake_case)]
    pub(crate) fn sign_prehashed<'a, D>(
        &self,
        prehashed_message: D,
        verifying_key: &VerifyingKey,
        context: Option<&'a [u8]>,
    ) -> Result<Signature, SignatureError>
    where
        D: Digest<OutputSize = U64>,
    {
        let mut h: Sha512;
        let mut prehash: [u8; 64] = [0u8; 64];

        let ctx: &[u8] = context.unwrap_or(b""); // By default, the context is an empty string.

        if ctx.len() > 255 {
            return Err(SignatureError::from(InternalError::PrehashedContextLength));
        }

        let ctx_len: u8 = ctx.len() as u8;

        // Get the result of the pre-hashed message.
        prehash.copy_from_slice(prehashed_message.finalize().as_slice());

        // This is the dumbest, ten-years-late, non-admission of fucking up the
        // domain separation I have ever seen.  Why am I still required to put
        // the upper half "prefix" of the hashed "secret key" in here?  Why
        // can't the user just supply their own nonce and decide for themselves
        // whether or not they want a deterministic signature scheme?  Why does
        // the message go into what's ostensibly the signature domain separation
        // hash?  Why wasn't there always a way to provide a context string?
        //
        // ...
        //
        // This is a really fucking stupid bandaid, and the damned scheme is
        // still bleeding from malleability, for fuck's sake.
        h = Sha512::new()
            .chain_update(b"SigEd25519 no Ed25519 collisions")
            .chain_update([1]) // Ed25519ph
            .chain_update([ctx_len])
            .chain_update(ctx)
            .chain_update(self.nonce)
            .chain_update(&prehash[..]);

        let r = Scalar::from_hash(h);
        let R: CompressedEdwardsY = EdwardsPoint::mul_base(&r).compress();

        h = Sha512::new()
            .chain_update(b"SigEd25519 no Ed25519 collisions")
            .chain_update([1]) // Ed25519ph
            .chain_update([ctx_len])
            .chain_update(ctx)
            .chain_update(R.as_bytes())
            .chain_update(verifying_key.as_bytes())
            .chain_update(&prehash[..]);

        let k = Scalar::from_hash(h);
        let s: Scalar = (k * self.key) + r;

        Ok(InternalSignature { R, s }.into())
    }
}
