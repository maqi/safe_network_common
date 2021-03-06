// Copyright 2016 MaidSafe.net limited.
//
// This SAFE Network Software is licensed to you under (1) the MaidSafe.net Commercial License,
// version 1.0 or later, or (2) The General Public License (GPL), version 3, depending on which
// licence you accepted on initial access to the Software (the "Licences").
//
// By contributing code to the SAFE Network Software, or to this project generally, you agree to be
// bound by the terms of the MaidSafe Contributor Agreement, version 1.0.  This, along with the
// Licenses can be found in the root directory of this project at LICENSE, COPYING and CONTRIBUTOR.
//
// Unless required by applicable law or agreed to in writing, the SAFE Network Software distributed
// under the GPL Licence is distributed on an "AS IS" BASIS, WITHOUT WARRANTIES OR CONDITIONS OF ANY
// KIND, either express or implied.
//
// Please review the Licences for the specific language governing permissions and limitations
// relating to use of the SAFE Network Software.

/// Maximum allowed length for a [header's `metadata`](struct.MpidHeader.html#method.new) (128
/// bytes).
pub const MAX_HEADER_METADATA_SIZE: usize = 128;  // bytes

use std::fmt::{self, Debug, Formatter};
use std::sync::{Once, ONCE_INIT};

use maidsafe_utilities::serialisation::serialise;
use rand::{self, Rng};
use sodiumoxide;
use sodiumoxide::crypto::hash::sha512;
use sodiumoxide::crypto::sign::{self, PublicKey, SecretKey, Signature};
use super::{Error, GUID_SIZE};
use xor_name::XorName;
use messaging;

static INITIALISE_SODIUMOXIDE: Once = ONCE_INIT;
static mut sodiumoxide_init_result: bool = false;

#[derive(PartialEq, Eq, Hash, Clone, RustcDecodable, RustcEncodable)]
struct Detail {
    sender: XorName,
    guid: [u8; GUID_SIZE],
    metadata: Vec<u8>,
}

/// Minimal information about a given message which can be used as a notification to the receiver.
#[derive(PartialEq, Eq, Hash, Clone, RustcDecodable, RustcEncodable)]
pub struct MpidHeader {
    detail: Detail,
    signature: Signature,
}

impl MpidHeader {
    /// Constructor.
    ///
    /// Each new `MpidHeader` will have a random unique identifier assigned to it, accessed via the
    /// [`guid()`](#method.guid) getter.
    ///
    /// `sender` represents the name of the original creator of the message.
    ///
    /// `metadata` is arbitrary, user-supplied information which must not exceed
    /// [`MAX_HEADER_METADATA_SIZE`](constant.MAX_HEADER_METADATA_SIZE.html).  It can be empty if
    /// desired.
    ///
    /// `secret_key` will be used to generate a signature of `sender`, `guid` and `metadata`.
    ///
    /// An error will be returned if `metadata` exceeds `MAX_HEADER_METADATA_SIZE` or if
    /// serialisation during the signing process fails.
    pub fn new(sender: XorName, metadata: Vec<u8>, secret_key: &SecretKey) -> Result<MpidHeader, Error> {
        assert!(Self::initialise_sodiumoxide());
        if metadata.len() > MAX_HEADER_METADATA_SIZE {
            return Err(Error::MetadataTooLarge);
        }

        let mut detail = Detail {
            sender: sender,
            guid: [0u8; GUID_SIZE],
            metadata: metadata,
        };
        rand::thread_rng().fill_bytes(&mut detail.guid);

        let encoded = try!(serialise(&detail));
        Ok(MpidHeader {
            detail: detail,
            signature: sign::sign_detached(&encoded, secret_key),
        })
    }

    /// The name of the original creator of the message.
    pub fn sender(&self) -> &XorName {
        &self.detail.sender
    }

    /// A unique identifier generated randomly when calling `new()`.
    pub fn guid(&self) -> &[u8; GUID_SIZE] {
        &self.detail.guid
    }

    /// Arbitrary, user-supplied information.
    pub fn metadata(&self) -> &Vec<u8> {
        &self.detail.metadata
    }

    /// The signature of `sender`, `guid` and `metadata`, created when calling `new()`.
    pub fn signature(&self) -> &Signature {
        &self.signature
    }

    /// The name of the header.  This is a relatively expensive getter - the name is the SHA512 hash
    /// of the serialised header, so its use should be minimised.
    pub fn name(&self) -> Result<XorName, Error> {
        let encoded = try!(serialise(self));
        Ok(XorName(sha512::hash(&encoded[..]).0))
    }

    /// Validates the header's signature against the provided `PublicKey`.
    pub fn verify(&self, public_key: &PublicKey) -> bool {
        match serialise(&self.detail) {
            Ok(encoded) => sign::verify_detached(&self.signature, &encoded, public_key),
            Err(_) => false,
        }
    }

    #[allow(unsafe_code)]
    fn initialise_sodiumoxide() -> bool {
        unsafe {
            INITIALISE_SODIUMOXIDE.call_once(|| {
                sodiumoxide_init_result = sodiumoxide::init();
            });
            sodiumoxide_init_result
        }
    }
}

impl Debug for MpidHeader {
    fn fmt(&self, formatter: &mut Formatter) -> Result<(), fmt::Error> {
        write!(formatter,
               "MpidHeader {{ sender: {:?}, guid: {}, metadata: {}, signature: {} }}",
               self.detail.sender,
               messaging::format_binary_array(&self.detail.guid),
               messaging::format_binary_array(&self.detail.metadata),
               messaging::format_binary_array(&self.signature))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use rand;
    use sodiumoxide::crypto::sign;
    use xor_name::XorName;
    use messaging;

    #[test]
    fn full() {
        let (mut public_key, secret_key) = sign::gen_keypair();
        let sender: XorName = rand::random();

        // Check with metadata which is empty, then at size limit, then just above limit.
        {
            let header = unwrap_result!(MpidHeader::new(sender.clone(), vec![], &secret_key));
            assert!(header.metadata().is_empty());
        }
        let mut metadata = messaging::generate_random_bytes(MAX_HEADER_METADATA_SIZE);
        let header = unwrap_result!(MpidHeader::new(sender.clone(), metadata.clone(), &secret_key));
        assert!(*header.metadata() == metadata);
        metadata.push(0);
        assert!(MpidHeader::new(sender.clone(), metadata.clone(), &secret_key).is_err());
        let _ = metadata.pop();

        // Check verify function with a valid and invalid key
        assert!(header.verify(&public_key));
        if public_key.0[0] == 255 {
            public_key.0[0] += 1;
        } else {
            public_key.0[0] = 0;
        }
        assert!(!header.verify(&public_key));

        // Check that identically-constructed headers retain identical sender and metadata, but have
        // different GUIDs and signatures.
        let header1 = unwrap_result!(MpidHeader::new(sender.clone(), metadata.clone(), &secret_key));
        let header2 = unwrap_result!(MpidHeader::new(sender.clone(), metadata.clone(), &secret_key));
        assert!(header1 != header2);
        assert_eq!(*header1.sender(), sender);
        assert_eq!(header1.sender(), header2.sender());
        assert_eq!(*header1.metadata(), metadata);
        assert_eq!(header1.metadata(), header2.metadata());
        assert!(header1.guid() != header2.guid());
        assert!(header1.signature() != header2.signature());
        let name1 = unwrap_result!(header1.name());
        let name2 = unwrap_result!(header2.name());
        assert!(name1 != name2);
    }
}
