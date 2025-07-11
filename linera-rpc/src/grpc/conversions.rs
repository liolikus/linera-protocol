// Copyright (c) Zefchain Labs, Inc.
// SPDX-License-Identifier: Apache-2.0

use linera_base::{
    crypto::{
        AccountPublicKey, AccountSignature, CryptoError, CryptoHash, ValidatorPublicKey,
        ValidatorSignature,
    },
    data_types::{BlobContent, BlockHeight, NetworkDescription},
    ensure,
    identifiers::{AccountOwner, BlobId, ChainId},
};
use linera_chain::{
    data_types::{BlockProposal, LiteValue, ProposalContent},
    types::{
        Certificate, CertificateKind, ConfirmedBlock, ConfirmedBlockCertificate, LiteCertificate,
        Timeout, TimeoutCertificate, ValidatedBlock, ValidatedBlockCertificate,
    },
};
use linera_core::{
    data_types::{ChainInfoQuery, ChainInfoResponse, CrossChainRequest},
    node::NodeError,
    worker::Notification,
};
use thiserror::Error;
use tonic::{Code, Status};

use super::api::{self, PendingBlobRequest};
use crate::{
    HandleConfirmedCertificateRequest, HandleLiteCertRequest, HandleTimeoutCertificateRequest,
    HandleValidatedCertificateRequest,
};

#[derive(Error, Debug)]
pub enum GrpcProtoConversionError {
    #[error(transparent)]
    BincodeError(#[from] bincode::Error),
    #[error("Conversion failed due to missing field")]
    MissingField,
    #[error("Signature error: {0}")]
    SignatureError(ed25519_dalek::SignatureError),
    #[error("Cryptographic error: {0}")]
    CryptoError(#[from] CryptoError),
    #[error("Inconsistent outer/inner chain IDs")]
    InconsistentChainId,
    #[error("Unrecognized certificate type")]
    InvalidCertificateType,
}

impl From<ed25519_dalek::SignatureError> for GrpcProtoConversionError {
    fn from(signature_error: ed25519_dalek::SignatureError) -> Self {
        GrpcProtoConversionError::SignatureError(signature_error)
    }
}

/// Extracts an optional field from a Proto type and tries to map it.
fn try_proto_convert<S, T>(t: Option<T>) -> Result<S, GrpcProtoConversionError>
where
    T: TryInto<S, Error = GrpcProtoConversionError>,
{
    t.ok_or(GrpcProtoConversionError::MissingField)?.try_into()
}

impl From<GrpcProtoConversionError> for Status {
    fn from(error: GrpcProtoConversionError) -> Self {
        Status::new(Code::InvalidArgument, error.to_string())
    }
}

impl From<GrpcProtoConversionError> for NodeError {
    fn from(error: GrpcProtoConversionError) -> Self {
        NodeError::GrpcError {
            error: error.to_string(),
        }
    }
}

impl From<linera_version::CrateVersion> for api::CrateVersion {
    fn from(
        linera_version::CrateVersion {
            major,
            minor,
            patch,
        }: linera_version::CrateVersion,
    ) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl From<api::CrateVersion> for linera_version::CrateVersion {
    fn from(
        api::CrateVersion {
            major,
            minor,
            patch,
        }: api::CrateVersion,
    ) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }
}

impl From<linera_version::VersionInfo> for api::VersionInfo {
    fn from(version_info: linera_version::VersionInfo) -> api::VersionInfo {
        api::VersionInfo {
            crate_version: Some(version_info.crate_version.value.into()),
            git_commit: version_info.git_commit.into(),
            git_dirty: version_info.git_dirty,
            rpc_hash: version_info.rpc_hash.into(),
            graphql_hash: version_info.graphql_hash.into(),
            wit_hash: version_info.wit_hash.into(),
        }
    }
}

impl From<api::VersionInfo> for linera_version::VersionInfo {
    fn from(version_info: api::VersionInfo) -> linera_version::VersionInfo {
        linera_version::VersionInfo {
            crate_version: linera_version::Pretty::new(
                version_info
                    .crate_version
                    .unwrap_or(api::CrateVersion {
                        major: 0,
                        minor: 0,
                        patch: 0,
                    })
                    .into(),
            ),
            git_commit: version_info.git_commit.into(),
            git_dirty: version_info.git_dirty,
            rpc_hash: version_info.rpc_hash.into(),
            graphql_hash: version_info.graphql_hash.into(),
            wit_hash: version_info.wit_hash.into(),
        }
    }
}

impl From<NetworkDescription> for api::NetworkDescription {
    fn from(
        NetworkDescription {
            name,
            genesis_config_hash,
            genesis_timestamp,
            genesis_committee_blob_hash,
            admin_chain_id,
        }: NetworkDescription,
    ) -> Self {
        Self {
            name,
            genesis_config_hash: Some(genesis_config_hash.into()),
            genesis_timestamp: genesis_timestamp.micros(),
            admin_chain_id: Some(admin_chain_id.into()),
            genesis_committee_blob_hash: Some(genesis_committee_blob_hash.into()),
        }
    }
}

impl TryFrom<api::NetworkDescription> for NetworkDescription {
    type Error = GrpcProtoConversionError;

    fn try_from(
        api::NetworkDescription {
            name,
            genesis_config_hash,
            genesis_timestamp,
            genesis_committee_blob_hash,
            admin_chain_id,
        }: api::NetworkDescription,
    ) -> Result<Self, Self::Error> {
        Ok(Self {
            name,
            genesis_config_hash: try_proto_convert(genesis_config_hash)?,
            genesis_timestamp: genesis_timestamp.into(),
            admin_chain_id: try_proto_convert(admin_chain_id)?,
            genesis_committee_blob_hash: try_proto_convert(genesis_committee_blob_hash)?,
        })
    }
}

impl TryFrom<Notification> for api::Notification {
    type Error = GrpcProtoConversionError;

    fn try_from(notification: Notification) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(notification.chain_id.into()),
            reason: bincode::serialize(&notification.reason)?,
        })
    }
}

impl TryFrom<api::Notification> for Option<Notification> {
    type Error = GrpcProtoConversionError;

    fn try_from(notification: api::Notification) -> Result<Self, Self::Error> {
        if notification.chain_id.is_none() && notification.reason.is_empty() {
            Ok(None)
        } else {
            Ok(Some(Notification {
                chain_id: try_proto_convert(notification.chain_id)?,
                reason: bincode::deserialize(&notification.reason)?,
            }))
        }
    }
}

impl TryFrom<ChainInfoResponse> for api::ChainInfoResult {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_info_response: ChainInfoResponse) -> Result<Self, Self::Error> {
        let response = chain_info_response.try_into()?;
        Ok(api::ChainInfoResult {
            inner: Some(api::chain_info_result::Inner::ChainInfoResponse(response)),
        })
    }
}

impl TryFrom<NodeError> for api::ChainInfoResult {
    type Error = GrpcProtoConversionError;

    fn try_from(node_error: NodeError) -> Result<Self, Self::Error> {
        let error = bincode::serialize(&node_error)?;
        Ok(api::ChainInfoResult {
            inner: Some(api::chain_info_result::Inner::Error(error)),
        })
    }
}

impl TryFrom<BlockProposal> for api::BlockProposal {
    type Error = GrpcProtoConversionError;

    fn try_from(block_proposal: BlockProposal) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(block_proposal.content.block.chain_id.into()),
            content: bincode::serialize(&block_proposal.content)?,
            owner: Some(block_proposal.owner().try_into()?),
            signature: Some(block_proposal.signature.into()),
            original_proposal: block_proposal
                .original_proposal
                .map(|cert| bincode::serialize(&cert))
                .transpose()?,
        })
    }
}

impl TryFrom<api::BlockProposal> for BlockProposal {
    type Error = GrpcProtoConversionError;

    fn try_from(block_proposal: api::BlockProposal) -> Result<Self, Self::Error> {
        let content: ProposalContent = bincode::deserialize(&block_proposal.content)?;
        ensure!(
            Some(content.block.chain_id.into()) == block_proposal.chain_id,
            GrpcProtoConversionError::InconsistentChainId
        );
        Ok(Self {
            content,
            signature: try_proto_convert(block_proposal.signature)?,
            original_proposal: block_proposal
                .original_proposal
                .map(|bytes| bincode::deserialize(&bytes))
                .transpose()?,
        })
    }
}

impl TryFrom<api::CrossChainRequest> for CrossChainRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(cross_chain_request: api::CrossChainRequest) -> Result<Self, Self::Error> {
        use api::cross_chain_request::Inner;

        let ccr = match cross_chain_request
            .inner
            .ok_or(GrpcProtoConversionError::MissingField)?
        {
            Inner::UpdateRecipient(api::UpdateRecipient {
                sender,
                recipient,
                bundles,
            }) => CrossChainRequest::UpdateRecipient {
                sender: try_proto_convert(sender)?,
                recipient: try_proto_convert(recipient)?,
                bundles: bincode::deserialize(&bundles)?,
            },
            Inner::ConfirmUpdatedRecipient(api::ConfirmUpdatedRecipient {
                sender,
                recipient,
                latest_height,
            }) => CrossChainRequest::ConfirmUpdatedRecipient {
                sender: try_proto_convert(sender)?,
                recipient: try_proto_convert(recipient)?,
                latest_height: latest_height
                    .ok_or(GrpcProtoConversionError::MissingField)?
                    .into(),
            },
        };
        Ok(ccr)
    }
}

impl TryFrom<CrossChainRequest> for api::CrossChainRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(cross_chain_request: CrossChainRequest) -> Result<Self, Self::Error> {
        use api::cross_chain_request::Inner;

        let inner = match cross_chain_request {
            CrossChainRequest::UpdateRecipient {
                sender,
                recipient,
                bundles,
            } => Inner::UpdateRecipient(api::UpdateRecipient {
                sender: Some(sender.into()),
                recipient: Some(recipient.into()),
                bundles: bincode::serialize(&bundles)?,
            }),
            CrossChainRequest::ConfirmUpdatedRecipient {
                sender,
                recipient,
                latest_height,
            } => Inner::ConfirmUpdatedRecipient(api::ConfirmUpdatedRecipient {
                sender: Some(sender.into()),
                recipient: Some(recipient.into()),
                latest_height: Some(latest_height.into()),
            }),
        };
        Ok(Self { inner: Some(inner) })
    }
}

impl TryFrom<api::LiteCertificate> for HandleLiteCertRequest<'_> {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: api::LiteCertificate) -> Result<Self, Self::Error> {
        let kind = if certificate.kind == api::CertificateKind::Validated as i32 {
            CertificateKind::Validated
        } else if certificate.kind == api::CertificateKind::Confirmed as i32 {
            CertificateKind::Confirmed
        } else if certificate.kind == api::CertificateKind::Timeout as i32 {
            CertificateKind::Timeout
        } else {
            return Err(GrpcProtoConversionError::InvalidCertificateType);
        };

        let value = LiteValue {
            value_hash: CryptoHash::try_from(certificate.hash.as_slice())?,
            chain_id: try_proto_convert(certificate.chain_id)?,
            kind,
        };
        let signatures = bincode::deserialize(&certificate.signatures)?;
        let round = bincode::deserialize(&certificate.round)?;
        Ok(Self {
            certificate: LiteCertificate::new(value, round, signatures),
            wait_for_outgoing_messages: certificate.wait_for_outgoing_messages,
        })
    }
}

impl TryFrom<HandleLiteCertRequest<'_>> for api::LiteCertificate {
    type Error = GrpcProtoConversionError;

    fn try_from(request: HandleLiteCertRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            hash: request.certificate.value.value_hash.as_bytes().to_vec(),
            round: bincode::serialize(&request.certificate.round)?,
            chain_id: Some(request.certificate.value.chain_id.into()),
            signatures: bincode::serialize(&request.certificate.signatures)?,
            wait_for_outgoing_messages: request.wait_for_outgoing_messages,
            kind: request.certificate.value.kind as i32,
        })
    }
}

impl TryFrom<api::HandleTimeoutCertificateRequest> for HandleTimeoutCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(cert_request: api::HandleTimeoutCertificateRequest) -> Result<Self, Self::Error> {
        let certificate: TimeoutCertificate = cert_request
            .certificate
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        let req_chain_id: ChainId = cert_request
            .chain_id
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        ensure!(
            certificate.inner().chain_id() == req_chain_id,
            GrpcProtoConversionError::InconsistentChainId
        );
        Ok(HandleTimeoutCertificateRequest { certificate })
    }
}

impl TryFrom<api::HandleValidatedCertificateRequest> for HandleValidatedCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(cert_request: api::HandleValidatedCertificateRequest) -> Result<Self, Self::Error> {
        let certificate: ValidatedBlockCertificate = cert_request
            .certificate
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        let req_chain_id: ChainId = cert_request
            .chain_id
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        ensure!(
            certificate.inner().chain_id() == req_chain_id,
            GrpcProtoConversionError::InconsistentChainId
        );
        Ok(HandleValidatedCertificateRequest { certificate })
    }
}

impl TryFrom<api::HandleConfirmedCertificateRequest> for HandleConfirmedCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(cert_request: api::HandleConfirmedCertificateRequest) -> Result<Self, Self::Error> {
        let certificate: ConfirmedBlockCertificate = cert_request
            .certificate
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        let req_chain_id: ChainId = cert_request
            .chain_id
            .ok_or(GrpcProtoConversionError::MissingField)?
            .try_into()?;

        ensure!(
            certificate.inner().chain_id() == req_chain_id,
            GrpcProtoConversionError::InconsistentChainId
        );
        Ok(HandleConfirmedCertificateRequest {
            certificate,
            wait_for_outgoing_messages: cert_request.wait_for_outgoing_messages,
        })
    }
}

impl TryFrom<HandleConfirmedCertificateRequest> for api::HandleConfirmedCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(request: HandleConfirmedCertificateRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(request.certificate.inner().chain_id().into()),
            certificate: Some(request.certificate.try_into()?),
            wait_for_outgoing_messages: request.wait_for_outgoing_messages,
        })
    }
}

impl TryFrom<HandleValidatedCertificateRequest> for api::HandleValidatedCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(request: HandleValidatedCertificateRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(request.certificate.inner().chain_id().into()),
            certificate: Some(request.certificate.try_into()?),
        })
    }
}

impl TryFrom<HandleTimeoutCertificateRequest> for api::HandleTimeoutCertificateRequest {
    type Error = GrpcProtoConversionError;

    fn try_from(request: HandleTimeoutCertificateRequest) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(request.certificate.inner().chain_id().into()),
            certificate: Some(request.certificate.try_into()?),
        })
    }
}

impl TryFrom<api::Certificate> for TimeoutCertificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: api::Certificate) -> Result<Self, Self::Error> {
        let round = bincode::deserialize(&certificate.round)?;
        let signatures = bincode::deserialize(&certificate.signatures)?;
        let cert_type = certificate.kind;

        if cert_type == api::CertificateKind::Timeout as i32 {
            let value: Timeout = bincode::deserialize(&certificate.value)?;
            Ok(TimeoutCertificate::new(value, round, signatures))
        } else {
            Err(GrpcProtoConversionError::InvalidCertificateType)
        }
    }
}

impl TryFrom<api::Certificate> for ValidatedBlockCertificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: api::Certificate) -> Result<Self, Self::Error> {
        let round = bincode::deserialize(&certificate.round)?;
        let signatures = bincode::deserialize(&certificate.signatures)?;
        let cert_type = certificate.kind;

        if cert_type == api::CertificateKind::Validated as i32 {
            let value: ValidatedBlock = bincode::deserialize(&certificate.value)?;
            Ok(ValidatedBlockCertificate::new(value, round, signatures))
        } else {
            Err(GrpcProtoConversionError::InvalidCertificateType)
        }
    }
}

impl TryFrom<api::Certificate> for ConfirmedBlockCertificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: api::Certificate) -> Result<Self, Self::Error> {
        let round = bincode::deserialize(&certificate.round)?;
        let signatures = bincode::deserialize(&certificate.signatures)?;
        let cert_type = certificate.kind;

        if cert_type == api::CertificateKind::Confirmed as i32 {
            let value: ConfirmedBlock = bincode::deserialize(&certificate.value)?;
            Ok(ConfirmedBlockCertificate::new(value, round, signatures))
        } else {
            Err(GrpcProtoConversionError::InvalidCertificateType)
        }
    }
}

impl TryFrom<TimeoutCertificate> for api::Certificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: TimeoutCertificate) -> Result<Self, Self::Error> {
        let round = bincode::serialize(&certificate.round)?;
        let signatures = bincode::serialize(certificate.signatures())?;

        let value = bincode::serialize(certificate.value())?;

        Ok(Self {
            value,
            round,
            signatures,
            kind: api::CertificateKind::Timeout as i32,
        })
    }
}

impl TryFrom<ConfirmedBlockCertificate> for api::Certificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: ConfirmedBlockCertificate) -> Result<Self, Self::Error> {
        let round = bincode::serialize(&certificate.round)?;
        let signatures = bincode::serialize(certificate.signatures())?;

        let value = bincode::serialize(certificate.value())?;

        Ok(Self {
            value,
            round,
            signatures,
            kind: api::CertificateKind::Confirmed as i32,
        })
    }
}

impl TryFrom<ValidatedBlockCertificate> for api::Certificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: ValidatedBlockCertificate) -> Result<Self, Self::Error> {
        let round = bincode::serialize(&certificate.round)?;
        let signatures = bincode::serialize(certificate.signatures())?;

        let value = bincode::serialize(certificate.value())?;

        Ok(Self {
            value,
            round,
            signatures,
            kind: api::CertificateKind::Validated as i32,
        })
    }
}

impl TryFrom<api::ChainInfoQuery> for ChainInfoQuery {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_info_query: api::ChainInfoQuery) -> Result<Self, Self::Error> {
        let request_sent_certificate_hashes_in_range = chain_info_query
            .request_sent_certificate_hashes_in_range
            .map(|range| bincode::deserialize(&range))
            .transpose()?;

        Ok(Self {
            request_committees: chain_info_query.request_committees,
            request_owner_balance: try_proto_convert(chain_info_query.request_owner_balance)?,
            request_pending_message_bundles: chain_info_query.request_pending_message_bundles,
            chain_id: try_proto_convert(chain_info_query.chain_id)?,
            request_sent_certificate_hashes_in_range,
            request_received_log_excluding_first_n: chain_info_query
                .request_received_log_excluding_first_n,
            test_next_block_height: chain_info_query.test_next_block_height.map(Into::into),
            request_manager_values: chain_info_query.request_manager_values,
            request_leader_timeout: chain_info_query.request_leader_timeout,
            request_fallback: chain_info_query.request_fallback,
        })
    }
}

impl TryFrom<ChainInfoQuery> for api::ChainInfoQuery {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_info_query: ChainInfoQuery) -> Result<Self, Self::Error> {
        let request_sent_certificate_hashes_in_range = chain_info_query
            .request_sent_certificate_hashes_in_range
            .map(|range| bincode::serialize(&range))
            .transpose()?;
        let request_owner_balance = Some(chain_info_query.request_owner_balance.try_into()?);

        Ok(Self {
            chain_id: Some(chain_info_query.chain_id.into()),
            request_committees: chain_info_query.request_committees,
            request_owner_balance,
            request_pending_message_bundles: chain_info_query.request_pending_message_bundles,
            test_next_block_height: chain_info_query.test_next_block_height.map(Into::into),
            request_sent_certificate_hashes_in_range,
            request_received_log_excluding_first_n: chain_info_query
                .request_received_log_excluding_first_n,
            request_manager_values: chain_info_query.request_manager_values,
            request_leader_timeout: chain_info_query.request_leader_timeout,
            request_fallback: chain_info_query.request_fallback,
        })
    }
}

impl From<ChainId> for api::ChainId {
    fn from(chain_id: ChainId) -> Self {
        Self {
            bytes: chain_id.0.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<api::ChainId> for ChainId {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_id: api::ChainId) -> Result<Self, Self::Error> {
        Ok(ChainId::try_from(chain_id.bytes.as_slice())?)
    }
}

impl From<AccountPublicKey> for api::AccountPublicKey {
    fn from(public_key: AccountPublicKey) -> Self {
        Self {
            bytes: public_key.as_bytes(),
        }
    }
}

impl From<ValidatorPublicKey> for api::ValidatorPublicKey {
    fn from(public_key: ValidatorPublicKey) -> Self {
        Self {
            bytes: public_key.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<api::ValidatorPublicKey> for ValidatorPublicKey {
    type Error = GrpcProtoConversionError;

    fn try_from(public_key: api::ValidatorPublicKey) -> Result<Self, Self::Error> {
        Ok(Self::from_bytes(public_key.bytes.as_slice())?)
    }
}

impl TryFrom<api::AccountPublicKey> for AccountPublicKey {
    type Error = GrpcProtoConversionError;

    fn try_from(public_key: api::AccountPublicKey) -> Result<Self, Self::Error> {
        Ok(Self::from_slice(public_key.bytes.as_slice())?)
    }
}

impl From<AccountSignature> for api::AccountSignature {
    fn from(signature: AccountSignature) -> Self {
        Self {
            bytes: signature.to_bytes(),
        }
    }
}

impl From<ValidatorSignature> for api::ValidatorSignature {
    fn from(signature: ValidatorSignature) -> Self {
        Self {
            bytes: signature.as_bytes().to_vec(),
        }
    }
}

impl TryFrom<api::ValidatorSignature> for ValidatorSignature {
    type Error = GrpcProtoConversionError;

    fn try_from(signature: api::ValidatorSignature) -> Result<Self, Self::Error> {
        Self::from_slice(signature.bytes.as_slice()).map_err(GrpcProtoConversionError::CryptoError)
    }
}

impl TryFrom<api::AccountSignature> for AccountSignature {
    type Error = GrpcProtoConversionError;

    fn try_from(signature: api::AccountSignature) -> Result<Self, Self::Error> {
        Ok(Self::from_slice(signature.bytes.as_slice())?)
    }
}

impl TryFrom<ChainInfoResponse> for api::ChainInfoResponse {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_info_response: ChainInfoResponse) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_info: bincode::serialize(&chain_info_response.info)?,
            signature: chain_info_response.signature.map(Into::into),
        })
    }
}

impl TryFrom<api::ChainInfoResponse> for ChainInfoResponse {
    type Error = GrpcProtoConversionError;

    fn try_from(chain_info_response: api::ChainInfoResponse) -> Result<Self, Self::Error> {
        let signature = chain_info_response
            .signature
            .map(TryInto::try_into)
            .transpose()?;
        let info = bincode::deserialize(chain_info_response.chain_info.as_slice())?;
        Ok(Self { info, signature })
    }
}

impl TryFrom<(ChainId, BlobId)> for api::PendingBlobRequest {
    type Error = GrpcProtoConversionError;

    fn try_from((chain_id, blob_id): (ChainId, BlobId)) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(chain_id.into()),
            blob_id: Some(blob_id.try_into()?),
        })
    }
}

impl TryFrom<api::PendingBlobRequest> for (ChainId, BlobId) {
    type Error = GrpcProtoConversionError;

    fn try_from(request: PendingBlobRequest) -> Result<Self, Self::Error> {
        Ok((
            try_proto_convert(request.chain_id)?,
            try_proto_convert(request.blob_id)?,
        ))
    }
}

impl TryFrom<(ChainId, BlobContent)> for api::HandlePendingBlobRequest {
    type Error = GrpcProtoConversionError;

    fn try_from((chain_id, blob_content): (ChainId, BlobContent)) -> Result<Self, Self::Error> {
        Ok(Self {
            chain_id: Some(chain_id.into()),
            blob: Some(blob_content.try_into()?),
        })
    }
}

impl TryFrom<api::HandlePendingBlobRequest> for (ChainId, BlobContent) {
    type Error = GrpcProtoConversionError;

    fn try_from(request: api::HandlePendingBlobRequest) -> Result<Self, Self::Error> {
        Ok((
            try_proto_convert(request.chain_id)?,
            try_proto_convert(request.blob)?,
        ))
    }
}

impl TryFrom<BlobContent> for api::PendingBlobResult {
    type Error = GrpcProtoConversionError;

    fn try_from(blob: BlobContent) -> Result<Self, Self::Error> {
        Ok(Self {
            inner: Some(api::pending_blob_result::Inner::Blob(blob.try_into()?)),
        })
    }
}

impl TryFrom<NodeError> for api::PendingBlobResult {
    type Error = GrpcProtoConversionError;

    fn try_from(node_error: NodeError) -> Result<Self, Self::Error> {
        let error = bincode::serialize(&node_error)?;
        Ok(api::PendingBlobResult {
            inner: Some(api::pending_blob_result::Inner::Error(error)),
        })
    }
}

impl From<BlockHeight> for api::BlockHeight {
    fn from(block_height: BlockHeight) -> Self {
        Self {
            height: block_height.0,
        }
    }
}

impl From<api::BlockHeight> for BlockHeight {
    fn from(block_height: api::BlockHeight) -> Self {
        Self(block_height.height)
    }
}

impl TryFrom<AccountOwner> for api::AccountOwner {
    type Error = GrpcProtoConversionError;

    fn try_from(account_owner: AccountOwner) -> Result<Self, Self::Error> {
        Ok(Self {
            bytes: bincode::serialize(&account_owner)?,
        })
    }
}

impl TryFrom<api::AccountOwner> for AccountOwner {
    type Error = GrpcProtoConversionError;

    fn try_from(account_owner: api::AccountOwner) -> Result<Self, Self::Error> {
        Ok(bincode::deserialize(&account_owner.bytes)?)
    }
}

impl TryFrom<api::BlobId> for BlobId {
    type Error = GrpcProtoConversionError;

    fn try_from(blob_id: api::BlobId) -> Result<Self, Self::Error> {
        Ok(bincode::deserialize(blob_id.bytes.as_slice())?)
    }
}

impl TryFrom<api::BlobIds> for Vec<BlobId> {
    type Error = GrpcProtoConversionError;

    fn try_from(blob_ids: api::BlobIds) -> Result<Self, Self::Error> {
        Ok(blob_ids
            .bytes
            .into_iter()
            .map(|x| bincode::deserialize(x.as_slice()))
            .collect::<Result<_, _>>()?)
    }
}

impl TryFrom<BlobId> for api::BlobId {
    type Error = GrpcProtoConversionError;

    fn try_from(blob_id: BlobId) -> Result<Self, Self::Error> {
        Ok(Self {
            bytes: bincode::serialize(&blob_id)?,
        })
    }
}

impl TryFrom<Vec<BlobId>> for api::BlobIds {
    type Error = GrpcProtoConversionError;

    fn try_from(blob_ids: Vec<BlobId>) -> Result<Self, Self::Error> {
        let bytes = blob_ids
            .into_iter()
            .map(|blob_id| bincode::serialize(&blob_id))
            .collect::<Result<_, _>>()?;
        Ok(Self { bytes })
    }
}

impl TryFrom<api::CryptoHash> for CryptoHash {
    type Error = GrpcProtoConversionError;

    fn try_from(hash: api::CryptoHash) -> Result<Self, Self::Error> {
        Ok(CryptoHash::try_from(hash.bytes.as_slice())?)
    }
}

impl TryFrom<BlobContent> for api::BlobContent {
    type Error = GrpcProtoConversionError;

    fn try_from(blob: BlobContent) -> Result<Self, Self::Error> {
        Ok(Self {
            bytes: bincode::serialize(&blob)?,
        })
    }
}

impl TryFrom<api::BlobContent> for BlobContent {
    type Error = GrpcProtoConversionError;

    fn try_from(blob: api::BlobContent) -> Result<Self, Self::Error> {
        Ok(bincode::deserialize(blob.bytes.as_slice())?)
    }
}

impl From<CryptoHash> for api::CryptoHash {
    fn from(hash: CryptoHash) -> Self {
        Self {
            bytes: hash.as_bytes().to_vec(),
        }
    }
}

impl From<Vec<CryptoHash>> for api::CertificatesBatchRequest {
    fn from(certs: Vec<CryptoHash>) -> Self {
        Self {
            hashes: certs.into_iter().map(Into::into).collect(),
        }
    }
}

impl TryFrom<Certificate> for api::Certificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: Certificate) -> Result<Self, Self::Error> {
        let round = bincode::serialize(&certificate.round())?;
        let signatures = bincode::serialize(certificate.signatures())?;

        let (kind, value) = match certificate {
            Certificate::Confirmed(confirmed) => (
                api::CertificateKind::Confirmed,
                bincode::serialize(confirmed.value())?,
            ),
            Certificate::Validated(validated) => (
                api::CertificateKind::Validated,
                bincode::serialize(validated.value())?,
            ),
            Certificate::Timeout(timeout) => (
                api::CertificateKind::Timeout,
                bincode::serialize(timeout.value())?,
            ),
        };

        Ok(Self {
            value,
            round,
            signatures,
            kind: kind as i32,
        })
    }
}

impl TryFrom<api::Certificate> for Certificate {
    type Error = GrpcProtoConversionError;

    fn try_from(certificate: api::Certificate) -> Result<Self, Self::Error> {
        let round = bincode::deserialize(&certificate.round)?;
        let signatures = bincode::deserialize(&certificate.signatures)?;

        let value = if certificate.kind == api::CertificateKind::Confirmed as i32 {
            let value: ConfirmedBlock = bincode::deserialize(&certificate.value)?;
            Certificate::Confirmed(ConfirmedBlockCertificate::new(value, round, signatures))
        } else if certificate.kind == api::CertificateKind::Validated as i32 {
            let value: ValidatedBlock = bincode::deserialize(&certificate.value)?;
            Certificate::Validated(ValidatedBlockCertificate::new(value, round, signatures))
        } else if certificate.kind == api::CertificateKind::Timeout as i32 {
            let value: Timeout = bincode::deserialize(&certificate.value)?;
            Certificate::Timeout(TimeoutCertificate::new(value, round, signatures))
        } else {
            return Err(GrpcProtoConversionError::InvalidCertificateType);
        };

        Ok(value)
    }
}

impl TryFrom<Vec<Certificate>> for api::CertificatesBatchResponse {
    type Error = GrpcProtoConversionError;

    fn try_from(certs: Vec<Certificate>) -> Result<Self, Self::Error> {
        Ok(Self {
            certificates: certs
                .into_iter()
                .map(TryInto::try_into)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl TryFrom<api::CertificatesBatchResponse> for Vec<Certificate> {
    type Error = GrpcProtoConversionError;

    fn try_from(response: api::CertificatesBatchResponse) -> Result<Self, Self::Error> {
        response
            .certificates
            .into_iter()
            .map(Certificate::try_from)
            .collect()
    }
}

#[cfg(test)]
pub mod tests {
    use std::{borrow::Cow, fmt::Debug};

    use linera_base::{
        crypto::{AccountSecretKey, BcsSignable, CryptoHash, Secp256k1SecretKey, ValidatorKeypair},
        data_types::{Amount, Blob, Epoch, Round, Timestamp},
    };
    use linera_chain::{
        data_types::{BlockExecutionOutcome, OriginalProposal, ProposedBlock},
        test::make_first_block,
        types::CertificateKind,
    };
    use linera_core::data_types::ChainInfo;
    use serde::{Deserialize, Serialize};

    use super::*;

    #[derive(Debug, Serialize, Deserialize)]
    struct Foo(String);

    impl BcsSignable<'_> for Foo {}

    fn dummy_chain_id(index: u32) -> ChainId {
        ChainId(CryptoHash::test_hash(format!("chain{}", index)))
    }

    fn get_block() -> ProposedBlock {
        make_first_block(dummy_chain_id(0))
    }

    /// A convenience function for testing. It converts a type into its
    /// RPC equivalent and back - asserting that the two are equal.
    fn round_trip_check<T, M>(value: T)
    where
        T: TryFrom<M> + Clone + Debug + Eq,
        M: TryFrom<T>,
        T::Error: Debug,
        M::Error: Debug,
    {
        let message = M::try_from(value.clone()).unwrap();
        assert_eq!(value, message.try_into().unwrap());
    }

    #[test]
    pub fn test_public_key() {
        let account_key = AccountSecretKey::generate().public();
        round_trip_check::<_, api::AccountPublicKey>(account_key);

        let validator_key = ValidatorKeypair::generate().public_key;
        round_trip_check::<_, api::ValidatorPublicKey>(validator_key);
    }

    #[test]
    pub fn test_signature() {
        let validator_key_pair = ValidatorKeypair::generate();
        let validator_signature =
            ValidatorSignature::new(&Foo("test".into()), &validator_key_pair.secret_key);
        round_trip_check::<_, api::ValidatorSignature>(validator_signature);

        let account_key_pair = AccountSecretKey::generate();
        let account_signature = account_key_pair.sign(&Foo("test".into()));
        round_trip_check::<_, api::AccountSignature>(account_signature);
    }

    #[test]
    pub fn test_owner() {
        let key_pair = AccountSecretKey::generate();
        let owner = AccountOwner::from(key_pair.public());
        round_trip_check::<_, api::AccountOwner>(owner);
    }

    #[test]
    pub fn test_block_height() {
        let block_height = BlockHeight::from(10);
        round_trip_check::<_, api::BlockHeight>(block_height);
    }

    #[test]
    pub fn test_chain_id() {
        let chain_id = dummy_chain_id(0);
        round_trip_check::<_, api::ChainId>(chain_id);
    }

    #[test]
    pub fn test_chain_info_response() {
        let chain_info = Box::new(ChainInfo {
            chain_id: dummy_chain_id(0),
            epoch: Epoch::ZERO,
            description: None,
            manager: Box::default(),
            chain_balance: Amount::ZERO,
            block_hash: None,
            timestamp: Timestamp::default(),
            next_block_height: BlockHeight::ZERO,
            state_hash: None,
            requested_committees: None,
            requested_owner_balance: None,
            requested_pending_message_bundles: vec![],
            requested_sent_certificate_hashes: vec![],
            count_received_log: 0,
            requested_received_log: vec![],
        });

        let chain_info_response_none = ChainInfoResponse {
            // `info` is bincode so no need to test conversions extensively
            info: chain_info.clone(),
            signature: None,
        };
        round_trip_check::<_, api::ChainInfoResponse>(chain_info_response_none);

        let chain_info_response_some = ChainInfoResponse {
            // `info` is bincode so no need to test conversions extensively
            info: chain_info,
            signature: Some(ValidatorSignature::new(
                &Foo("test".into()),
                &ValidatorKeypair::generate().secret_key,
            )),
        };
        round_trip_check::<_, api::ChainInfoResponse>(chain_info_response_some);
    }

    #[test]
    pub fn test_chain_info_query() {
        let chain_info_query_none = ChainInfoQuery::new(dummy_chain_id(0));
        round_trip_check::<_, api::ChainInfoQuery>(chain_info_query_none);

        let chain_info_query_some = ChainInfoQuery {
            chain_id: dummy_chain_id(0),
            test_next_block_height: Some(BlockHeight::from(10)),
            request_committees: false,
            request_owner_balance: AccountOwner::CHAIN,
            request_pending_message_bundles: false,
            request_sent_certificate_hashes_in_range: Some(
                linera_core::data_types::BlockHeightRange {
                    start: BlockHeight::from(3),
                    limit: Some(5),
                },
            ),
            request_received_log_excluding_first_n: None,
            request_manager_values: false,
            request_leader_timeout: false,
            request_fallback: true,
        };
        round_trip_check::<_, api::ChainInfoQuery>(chain_info_query_some);
    }

    #[test]
    pub fn test_pending_blob_request() {
        let chain_id = dummy_chain_id(2);
        let blob_id = Blob::new(BlobContent::new_data(*b"foo")).id();
        let pending_blob_request = (chain_id, blob_id);
        round_trip_check::<_, api::PendingBlobRequest>(pending_blob_request);
    }

    #[test]
    pub fn test_pending_blob_result() {
        let blob = BlobContent::new_data(*b"foo");
        round_trip_check::<_, api::PendingBlobResult>(blob);
    }

    #[test]
    pub fn test_handle_pending_blob_request() {
        let chain_id = dummy_chain_id(2);
        let blob_content = BlobContent::new_data(*b"foo");
        let pending_blob_request = (chain_id, blob_content);
        round_trip_check::<_, api::HandlePendingBlobRequest>(pending_blob_request);
    }

    #[test]
    pub fn test_lite_certificate() {
        let key_pair = ValidatorKeypair::generate();
        let certificate = LiteCertificate {
            value: LiteValue {
                value_hash: CryptoHash::new(&Foo("value".into())),
                chain_id: dummy_chain_id(0),
                kind: CertificateKind::Validated,
            },
            round: Round::MultiLeader(2),
            signatures: Cow::Owned(vec![(
                key_pair.public_key,
                ValidatorSignature::new(&Foo("test".into()), &key_pair.secret_key),
            )]),
        };
        let request = HandleLiteCertRequest {
            certificate,
            wait_for_outgoing_messages: true,
        };

        round_trip_check::<_, api::LiteCertificate>(request);
    }

    #[test]
    pub fn test_certificate() {
        let key_pair = ValidatorKeypair::generate();
        let certificate = ValidatedBlockCertificate::new(
            ValidatedBlock::new(
                BlockExecutionOutcome {
                    state_hash: CryptoHash::new(&Foo("test".into())),
                    ..BlockExecutionOutcome::default()
                }
                .with(get_block()),
            ),
            Round::MultiLeader(3),
            vec![(
                key_pair.public_key,
                ValidatorSignature::new(&Foo("test".into()), &key_pair.secret_key),
            )],
        );
        let request = HandleValidatedCertificateRequest { certificate };

        round_trip_check::<_, api::HandleValidatedCertificateRequest>(request);
    }

    #[test]
    pub fn test_cross_chain_request() {
        let cross_chain_request_update_recipient = CrossChainRequest::UpdateRecipient {
            sender: dummy_chain_id(0),
            recipient: dummy_chain_id(0),
            bundles: vec![],
        };
        round_trip_check::<_, api::CrossChainRequest>(cross_chain_request_update_recipient);

        let cross_chain_request_confirm_updated_recipient =
            CrossChainRequest::ConfirmUpdatedRecipient {
                sender: dummy_chain_id(0),
                recipient: dummy_chain_id(0),
                latest_height: BlockHeight(1),
            };
        round_trip_check::<_, api::CrossChainRequest>(
            cross_chain_request_confirm_updated_recipient,
        );
    }

    #[test]
    pub fn test_block_proposal() {
        let key_pair = ValidatorKeypair::generate();
        let outcome = BlockExecutionOutcome {
            state_hash: CryptoHash::new(&Foo("validated".into())),
            ..BlockExecutionOutcome::default()
        };
        let certificate = ValidatedBlockCertificate::new(
            ValidatedBlock::new(outcome.clone().with(get_block())),
            Round::SingleLeader(2),
            vec![(
                key_pair.public_key,
                ValidatorSignature::new(&Foo("signed".into()), &key_pair.secret_key),
            )],
        )
        .lite_certificate()
        .cloned();
        let key_pair = AccountSecretKey::Secp256k1(Secp256k1SecretKey::generate());
        let block_proposal = BlockProposal {
            content: ProposalContent {
                block: get_block(),
                round: Round::SingleLeader(4),
                outcome: Some(outcome),
            },
            signature: key_pair.sign(&Foo("test".into())),
            original_proposal: Some(OriginalProposal::Regular { certificate }),
        };

        round_trip_check::<_, api::BlockProposal>(block_proposal);
    }

    #[test]
    pub fn test_notification() {
        let notification = Notification {
            chain_id: dummy_chain_id(0),
            reason: linera_core::worker::Reason::NewBlock {
                height: BlockHeight(0),
                hash: CryptoHash::new(&Foo("".into())),
            },
        };
        let message = api::Notification::try_from(notification.clone()).unwrap();
        assert_eq!(
            Some(notification),
            Option::<Notification>::try_from(message).unwrap()
        );

        let ack = api::Notification::default();
        assert_eq!(None, Option::<Notification>::try_from(ack).unwrap());
    }
}
